//! One peer's side of a shared level, as a plain state machine: it takes peer changes, local
//! edits and messages, and leaves messages to send in its outbox. No sockets or Bevy here, so
//! the tests run whole rooms of simulated peers.
//!
//! The rule, as in the checkers game this follows: **the level changes only by sequenced
//! edits.** One peer, the host (the smallest peer id, so every peer agrees without asking), puts
//! all edits into one order and sends them to everyone; everyone applies them in that order and
//! so ends up with the same level. Unlike the checkers moves, an editor can't wait for a round
//! trip before it shows a drag, so local edits show at once: the level on screen is the
//! sequenced level with this peer's own not-yet-sequenced edits on top ([`Session::expected`]).
//!
//! Whose level a room keeps: a peer that has been in the room with others holds the room's
//! level, and a newcomer takes it. When two peers that each edited alone meet, the one that came
//! first keeps its level.
//!
//! When the host leaves, edits it had not passed on yet are lost with it, and the remaining
//! peers may have received different amounts of what it did pass on. So a new host sends
//! everyone its level, and everyone sends it their pending edits again; edits are idempotent,
//! so one that did get through before does no harm the second time.

use std::collections::{BTreeMap, HashMap, HashSet};

use super::protocol::{NetMsg, Op, apply, diff};
use crate::level::Level;

/// What the shared level went through, for the screen to catch up with.
#[derive(Debug, PartialEq)]
pub enum Remote {
    /// Someone else's edits were applied.
    Ops(Vec<Op>),
    /// The level was replaced wholesale (this peer joined a room, or the room took its level).
    Reset,
}

/// What a peer said about itself in its last Hello.
struct Intro {
    holds_room: bool,
    since: f64,
    host: String,
    level: Level,
}

pub struct Session {
    pub me: String,
    pub name: String,
    /// Other peers in the room, sorted.
    peers: Vec<String>,
    /// Names of the peers that introduced themselves.
    pub names: BTreeMap<String, String>,
    /// Peers that have met this peer under its current name, with it believing in the current host.
    greeted: HashSet<String>,
    /// The last Hello of each peer.
    intros: HashMap<String, Intro>,
    /// Host only: peers whose level is settled (they were sent the room's, or the room took theirs).
    settled: HashSet<String>,
    host: String,
    nonce: u32,
    /// The newest edit of each peer that went through a host, so a host never passes on one
    /// twice: after a host change, peers send their pending edits again, and one may have made
    /// it before. A stale copy would undo whatever edited the same element since.
    passed: HashMap<String, u32>,
    /// This peer's edits the host has not sequenced yet, oldest first.
    pending: Vec<(u32, Vec<Op>)>,
    /// Has been in the room with someone: its level is the room's.
    holds_room: bool,
    since: f64,
    /// The level as the host sequenced it.
    synced: Level,
    /// The level as this peer last saw it on screen: local edits are what changed since.
    shown: Level,
    outbox: Vec<(Vec<String>, NetMsg)>,
    changes: Vec<Remote>,
}

impl Session {
    /// Joins with `level` (whose elements must have unique ids) as this peer's own.
    pub fn new(me: String, name: String, level: &Level, since: f64) -> Self {
        Self {
            host: me.clone(),
            me,
            name,
            peers: vec![],
            names: BTreeMap::new(),
            greeted: HashSet::new(),
            intros: HashMap::new(),
            settled: HashSet::new(),
            nonce: 0,
            passed: HashMap::new(),
            pending: vec![],
            holds_room: false,
            since,
            synced: level.clone(),
            shown: level.clone(),
            outbox: vec![],
            changes: vec![],
        }
    }

    pub fn peers(&self) -> &[String] {
        &self.peers
    }

    pub fn is_host(&self) -> bool {
        self.host == self.me
    }

    /// Whether this peer has the room's level yet. Until then it edits only its own copy.
    pub fn holds_room(&self) -> bool {
        self.holds_room
    }

    /// Edits of this peer that are still on their way through the host.
    #[cfg(test)]
    pub fn unsequenced(&self) -> usize {
        self.pending.len()
    }

    /// The level to show: the sequenced one with this peer's pending edits on top.
    pub fn expected(&self) -> Level {
        let mut level = self.synced.clone();
        for (_, ops) in &self.pending {
            apply(&mut level, ops);
        }
        level
    }

    /// The level now on screen, after the caller replaced it with [`Session::expected`] or
    /// applied remote settings to it.
    pub fn shown_mut(&mut self) -> &mut Level {
        &mut self.shown
    }

    pub fn take_outbox(&mut self) -> Vec<(Vec<String>, NetMsg)> {
        std::mem::take(&mut self.outbox)
    }

    /// What happened to the shared level since the last call.
    pub fn take_changes(&mut self) -> Vec<Remote> {
        std::mem::take(&mut self.changes)
    }

    /// The other peers now connected. Elects the host. A new host settles everyone again: a
    /// host that left may have passed its last edits on to some peers and not others, and took
    /// along those it hadn't passed on. Guests greet a new host again and send it their pending
    /// edits.
    pub fn set_peers(&mut self, mut peers: Vec<String>) {
        peers.sort();
        if peers == self.peers {
            return;
        }
        self.peers = peers;
        let present = |p: &String| self.peers.contains(p);
        self.names.retain(|p, _| present(p));
        self.greeted.retain(present);
        self.intros.retain(|p, _| present(p));
        self.settled.retain(present);
        let host = self
            .peers
            .first()
            .filter(|first| **first < self.me)
            .unwrap_or(&self.me)
            .clone();
        if host == self.host {
            return;
        }
        self.host = host;
        if self.is_host() {
            for (nonce, ops) in std::mem::take(&mut self.pending) {
                let me = self.me.clone();
                self.sequence(me, nonce, ops);
            }
            self.settled.clear();
            let ready: Vec<String> = self
                .intros
                .iter()
                .filter(|(_, intro)| intro.host == self.me)
                .map(|(peer, _)| peer.clone())
                .collect();
            for peer in ready {
                self.settle(&peer);
            }
        } else {
            self.greeted.remove(&self.host);
            if self.holds_room {
                self.resubmit();
            }
        }
    }

    /// Takes a new name: everyone gets introduced again.
    pub fn rename(&mut self, name: String) {
        self.name = name;
        self.greeted.clear();
    }

    /// Names must be unique in a room, or nobody could tell two cursors apart. Pet names are
    /// drawn before peers meet, so two can clash; the peer with the larger id draws a new one.
    pub fn settle_name_clash(&mut self) -> Option<String> {
        let clash = self
            .names
            .iter()
            .any(|(peer, name)| *peer < self.me && *name == self.name);
        clash.then(|| {
            let taken: Vec<&str> = self.names.values().map(String::as_str).collect();
            let name = super::room::petname_avoiding(&taken);
            self.rename(name.clone());
            name
        })
    }

    /// Introduces this peer to everyone who hasn't met it under its current name (or since
    /// becoming the host).
    pub fn greet(&mut self) {
        let to: Vec<String> = self
            .peers
            .iter()
            .filter(|p| !self.greeted.contains(*p))
            .cloned()
            .collect();
        if to.is_empty() {
            return;
        }
        self.greeted.extend(to.iter().cloned());
        let hello = NetMsg::Hello {
            name: self.name.clone(),
            holds_room: self.holds_room,
            since: self.since,
            host: self.host.clone(),
            level: self.synced.clone(),
        };
        self.outbox.push((to, hello));
    }

    /// Takes whatever changed between the level last shown and `level` as this peer's edit.
    pub fn local_edit(&mut self, level: &Level) {
        let ops = diff(&self.shown, level);
        if ops.is_empty() {
            return;
        }
        self.shown = level.clone();
        self.nonce += 1;
        let nonce = self.nonce;
        if self.is_host() {
            let me = self.me.clone();
            self.sequence(me, nonce, ops);
        } else {
            // A newcomer keeps its edits until it knows whether the room takes its level.
            if self.holds_room {
                self.outbox.push((
                    vec![self.host.clone()],
                    NetMsg::Submit {
                        nonce,
                        ops: ops.clone(),
                    },
                ));
            }
            self.pending.push((nonce, ops));
        }
    }

    /// Host: applies edits and sends them to everyone, which puts them in order.
    fn sequence(&mut self, origin: String, nonce: u32, ops: Vec<Op>) {
        self.passed.insert(origin.clone(), nonce);
        apply(&mut self.synced, &ops);
        if !self.peers.is_empty() {
            self.outbox
                .push((self.peers.clone(), NetMsg::Sequenced { origin, nonce, ops }));
        }
    }

    fn resubmit(&mut self) {
        for (nonce, ops) in &self.pending {
            self.outbox.push((
                vec![self.host.clone()],
                NetMsg::Submit {
                    nonce: *nonce,
                    ops: ops.clone(),
                },
            ));
        }
    }

    /// Host: decides whose level `peer` and the room go on with, once per peer.
    fn settle(&mut self, peer: &str) {
        let Some(intro) = self.intros.get(peer) else {
            return;
        };
        if !self.is_host() || intro.host != self.me || !self.settled.insert(peer.to_string()) {
            return;
        }
        let theirs = !self.holds_room
            && (intro.holds_room || (intro.since, peer) < (self.since, self.me.as_str()));
        self.holds_room = true;
        if theirs {
            // The room takes the newcomer's level; everyone here starts from it.
            self.synced = intro.level.clone();
            self.settled.extend(self.peers.iter().cloned());
            self.outbox.push((
                self.peers.clone(),
                NetMsg::Snapshot {
                    origin: peer.to_string(),
                    level: self.synced.clone(),
                    passed: self.passed.clone().into_iter().collect(),
                },
            ));
            self.changes.push(Remote::Reset);
        } else {
            self.outbox.push((
                vec![peer.to_string()],
                NetMsg::Snapshot {
                    origin: self.me.clone(),
                    level: self.synced.clone(),
                    passed: self.passed.clone().into_iter().collect(),
                },
            ));
        }
    }

    /// Handles a message from `from`. Cursors are the caller's business.
    pub fn receive(&mut self, from: &str, msg: NetMsg) {
        match msg {
            NetMsg::Hello {
                name,
                holds_room,
                since,
                host,
                level,
            } => {
                self.names.insert(from.to_string(), name);
                // A peer greets again when it renames or takes this peer for a new host; either
                // way it is settled afresh, since it may have followed another host meanwhile.
                self.settled.remove(from);
                self.intros.insert(
                    from.to_string(),
                    Intro {
                        holds_room,
                        since,
                        host,
                        level,
                    },
                );
                self.settle(from);
            }
            NetMsg::Submit { nonce, ops } => {
                // Meant for a host this peer no longer is (or not yet): the sender sends it
                // again to whoever hosts. Or a copy of an edit that was passed on already.
                if !self.is_host() || self.passed.get(from).is_some_and(|&n| nonce <= n) {
                    return;
                }
                self.sequence(from.to_string(), nonce, ops.clone());
                self.changes.push(Remote::Ops(ops));
            }
            // Only the host's word counts. A host this peer no longer follows (it left, or a
            // peer with a smaller id arrived) may still be sending; the new host settles this
            // peer afresh once it greets it.
            NetMsg::Sequenced { .. } | NetMsg::Snapshot { .. } if from != self.host => {}
            NetMsg::Sequenced { origin, nonce, ops } => {
                self.passed.insert(origin.clone(), nonce);
                apply(&mut self.synced, &ops);
                if origin != self.me {
                    self.changes.push(Remote::Ops(ops));
                    return;
                }
                // This peer's own edit, normally the oldest pending one and already on screen.
                // Older ones a departed host had passed on before won't come back again.
                let next = self.pending.first().is_some_and(|(n, _)| *n == nonce);
                self.pending.retain(|(n, _)| *n > nonce);
                if !next {
                    self.changes.push(Remote::Ops(ops));
                }
            }
            NetMsg::Snapshot {
                origin,
                level,
                passed,
            } => {
                self.synced = level;
                self.passed.extend(passed);
                if let Some(&done) = self.passed.get(&self.me) {
                    self.pending.retain(|(n, _)| *n > done);
                }
                if self.holds_room || origin == self.me {
                    // A new host catching everyone up, or the room took this peer's level:
                    // this peer's later edits still apply on top.
                    self.resubmit();
                } else {
                    // Edits to the level this peer had before joining don't carry over.
                    self.pending.clear();
                }
                self.holds_room = true;
                self.changes.push(Remote::Reset);
            }
            NetMsg::Cursor { .. } => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::{Level, preset_lab, preset_pong, preset_tower};
    use crate::net::protocol::assign_ids;
    use glam::Vec2;

    fn with_ids(mut level: Level, base: u64) -> Level {
        let mut next = base;
        assign_ids(&mut level, || {
            next += 1;
            next
        });
        level
    }

    /// A room of simulated peers on a network that delivers in order, one message at a time.
    struct Net {
        peers: Vec<(Session, Level)>,
        wire: std::collections::VecDeque<(String, String, NetMsg)>,
        /// Connection changes a peer has yet to notice: (peer, other, connected). Empty unless
        /// a test asks for uneven views; then peers notice them one at a time.
        notices: Vec<(String, String, bool)>,
        uneven: bool,
    }

    impl Net {
        fn new() -> Self {
            Self {
                peers: vec![],
                wire: Default::default(),
                notices: vec![],
                uneven: false,
            }
        }

        fn join(&mut self, id: &str, level: Level, since: f64) {
            let session = Session::new(id.into(), id.into(), &level, since);
            if self.uneven {
                for (other, _) in &self.peers {
                    self.notices.push((id.into(), other.me.clone(), true));
                    self.notices.push((other.me.clone(), id.into(), true));
                }
                self.peers.push((session, level));
                return;
            }
            self.peers.push((session, level));
            self.connect();
        }

        fn leave(&mut self, id: &str) {
            self.peers.retain(|(s, _)| s.me != id);
            self.wire.retain(|(from, to, _)| from != id && to != id);
            if self.uneven {
                self.notices.retain(|(p, o, _)| p != id && o != id);
                for (other, _) in &self.peers {
                    if other.peers.iter().any(|p| p == id) {
                        self.notices.push((other.me.clone(), id.into(), false));
                    }
                }
                return;
            }
            self.connect();
        }

        fn connect(&mut self) {
            let ids: Vec<String> = self.peers.iter().map(|(s, _)| s.me.clone()).collect();
            for (session, level) in &mut self.peers {
                session.set_peers(ids.iter().filter(|p| **p != session.me).cloned().collect());
                session.greet();
                if !session.take_changes().is_empty() {
                    *level = session.expected();
                    *session.shown_mut() = level.clone();
                }
            }
            self.collect();
        }

        /// One peer notices one connection change.
        fn notice(&mut self, i: usize) {
            let (id, other, connected) = self.notices.remove(i);
            let (session, level) = self.peer(&id);
            let mut peers = session.peers.clone();
            peers.retain(|p| *p != other);
            if connected {
                peers.push(other);
            }
            session.set_peers(peers);
            session.greet();
            if !session.take_changes().is_empty() {
                *level = session.expected();
                *session.shown_mut() = level.clone();
            }
            self.collect();
        }

        fn peer(&mut self, id: &str) -> &mut (Session, Level) {
            self.peers.iter_mut().find(|(s, _)| s.me == id).unwrap()
        }

        fn edit(&mut self, id: &str, f: impl FnOnce(&mut Level)) {
            let (session, level) = self.peer(id);
            f(level);
            let mut next = 1_000_000 * (id.as_bytes()[0] as u64);
            assign_ids(level, || {
                next += 1;
                next
            });
            session.local_edit(level);
            self.collect();
        }

        fn collect(&mut self) {
            for (session, _) in &mut self.peers {
                for (to, msg) in session.take_outbox() {
                    for to in to {
                        self.wire.push_back((session.me.clone(), to, msg.clone()));
                    }
                }
            }
        }

        /// Delivers one message, as the app would: the screen catches up with what changed.
        fn step(&mut self) -> bool {
            let Some((from, to, msg)) = self.wire.pop_front() else {
                return false;
            };
            if !self.peers.iter().any(|(s, _)| s.me == to) {
                return true; // It left; the network drops what was still on its way.
            }
            let (session, level) = self.peer(&to);
            session.local_edit(level);
            session.receive(&from, msg);
            if !session.take_changes().is_empty() {
                *level = session.expected();
                *session.shown_mut() = level.clone();
            }
            self.collect();
            true
        }

        /// Whether a message can arrive yet: only once the recipient knows the sender is there
        /// (matchbox reports a peer as connected before any of its data can be read).
        fn deliverable(&self, i: usize) -> bool {
            let (from, to, _) = &self.wire[i];
            let head = !self
                .wire
                .iter()
                .take(i)
                .any(|(f, t, _)| f == from && t == to);
            let known = self
                .peers
                .iter()
                .find(|(s, _)| s.me == *to)
                .is_none_or(|(s, _)| s.peers.contains(from));
            head && known
        }

        fn deliver(&mut self, i: usize) {
            let message = self.wire.remove(i).unwrap();
            self.wire.push_front(message);
            self.step();
        }

        fn settle(&mut self) {
            loop {
                if let Some(i) = (0..self.wire.len()).find(|&i| self.deliverable(i)) {
                    self.deliver(i);
                    continue;
                }
                if self.notices.is_empty() {
                    break;
                }
                self.notice(0);
            }
        }

        fn assert_converged(&self) -> Level {
            let first = &self.peers[0].1;
            for (session, level) in &self.peers {
                assert_eq!(level, first, "{} differs", session.me);
                assert_eq!(session.unsequenced(), 0, "{} has pending edits", session.me);
            }
            first.clone()
        }
    }

    #[test]
    fn a_newcomer_takes_the_level_of_whoever_was_there_first() {
        let mut net = Net::new();
        net.join("b", with_ids(preset_tower(), 100), 1.0);
        net.edit("b", |l| l.gravity = 123.0);
        // "a" has the smaller id, so it hosts, but "b" was first.
        net.join("a", with_ids(preset_pong(), 200), 2.0);
        net.settle();
        let level = net.assert_converged();
        assert_eq!(level.gravity, 123.0);
        assert_eq!(level.name, preset_tower().name);
    }

    #[test]
    fn a_newcomer_takes_the_room_level_even_when_it_hosts() {
        let mut net = Net::new();
        net.join("m", with_ids(preset_tower(), 100), 1.0);
        net.join("n", with_ids(preset_pong(), 200), 2.0);
        net.settle();
        net.edit("n", |l| l.elements[0].pos = Vec2::new(5.0, 5.0));
        net.settle();
        // Arrives last but with the smallest id, so it is the host at once.
        net.join("a", with_ids(preset_lab(), 300), 0.5);
        net.settle();
        let level = net.assert_converged();
        assert_eq!(level.name, preset_tower().name);
        assert_eq!(level.elements[0].pos, Vec2::new(5.0, 5.0));
    }

    #[test]
    fn concurrent_edits_converge_and_both_survive() {
        let mut net = Net::new();
        net.join("a", with_ids(preset_lab(), 100), 1.0);
        net.join("b", Level::default(), 2.0);
        net.join("c", Level::default(), 3.0);
        net.settle();
        net.assert_converged();
        // Three people edit at once, before anything is delivered.
        net.edit("a", |l| l.elements[0].angle = 1.0);
        net.edit("b", |l| l.elements[1].angle = 2.0);
        net.edit("c", |l| l.elements[1].pos.x += 30.0);
        net.edit("c", |l| {
            l.elements.remove(2);
        });
        net.edit("b", |l| l.gravity = 7.0);
        net.settle();
        let level = net.assert_converged();
        assert_eq!(level.elements[0].angle, 1.0);
        assert_eq!(level.gravity, 7.0);
        assert_eq!(
            level.elements.len(),
            with_ids(preset_lab(), 0).elements.len() - 1
        );
    }

    #[test]
    fn edits_survive_the_host_leaving() {
        let mut net = Net::new();
        net.join("a", with_ids(preset_lab(), 100), 1.0);
        net.join("b", Level::default(), 2.0);
        net.join("c", Level::default(), 3.0);
        net.settle();
        // "b" submits to "a", which leaves before sequencing it.
        net.edit("b", |l| l.name = "renamed".into());
        net.leave("a");
        net.settle();
        let level = net.assert_converged();
        assert_eq!(level.name, "renamed");
        net.edit("c", |l| l.gravity = 1.0);
        net.settle();
        assert_eq!(net.assert_converged().gravity, 1.0);
    }

    #[test]
    fn a_host_leaving_halfway_through_passing_an_edit_on_loses_nothing() {
        let mut net = Net::new();
        net.join("a", with_ids(preset_lab(), 100), 1.0);
        net.join("b", Level::default(), 2.0);
        net.join("c", Level::default(), 3.0);
        net.settle();
        net.edit("b", |l| l.gravity = 42.0);
        net.step(); // b's edit reaches the host
        net.step(); // the host passes it on to b, but leaves before c gets it
        net.leave("a");
        net.settle();
        assert_eq!(net.assert_converged().gravity, 42.0);
    }

    /// Random edits by random peers, delivered in random interleavings (in order between any
    /// two peers, as WebRTC's reliable channel delivers), with peers coming and going.
    #[test]
    fn random_rooms_converge() {
        random_rooms(false);
    }

    /// The same, with every peer noticing others come and go at its own pace, so peers
    /// disagree for a while about who is in the room, and so about who hosts.
    #[test]
    fn random_rooms_converge_with_uneven_views() {
        random_rooms(true);
    }

    fn random_rooms(uneven: bool) {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut rand = move |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        for _ in 0..200 {
            let mut net = Net::new();
            net.uneven = uneven;
            let presets = [preset_lab(), preset_pong(), preset_tower()];
            let names = ["a", "b", "c", "d", "e"];
            let mut joined = 0;
            for step in 0..120 {
                match rand(10) {
                    0 if joined < names.len() => {
                        let level = with_ids(presets[rand(3)].clone(), 100 * (joined as u64 + 1));
                        net.join(names[joined], level, step as f64);
                        joined += 1;
                    }
                    6 if !net.notices.is_empty() => {
                        let i = rand(net.notices.len());
                        net.notice(i);
                    }
                    1 if net.peers.len() > 2 && rand(4) == 0 => {
                        let id = net.peers[rand(net.peers.len())].0.me.clone();
                        net.leave(&id);
                    }
                    2..=5 if !net.peers.is_empty() => {
                        let id = net.peers[rand(net.peers.len())].0.me.clone();
                        let (pick, value) = (rand(6), rand(1000) as f32);
                        net.edit(&id, |l| {
                            let n = l.elements.len();
                            match pick {
                                0 => l.gravity = value,
                                1 if n > 0 => l.elements[value as usize % n].pos.x = value,
                                2 if n > 1 => {
                                    l.elements.remove(value as usize % n);
                                }
                                3 if n > 0 => {
                                    let copy = l.elements[value as usize % n].clone();
                                    l.elements.push(copy);
                                }
                                4 if n > 1 => l.elements.swap(0, n - 1),
                                _ => l.name = format!("{value}"),
                            }
                        });
                    }
                    _ => {
                        // Deliver one message that is next in line between its two peers.
                        let ready: Vec<usize> = (0..net.wire.len())
                            .filter(|&i| net.deliverable(i))
                            .collect();
                        if !ready.is_empty() {
                            let i = ready[rand(ready.len())];
                            net.deliver(i);
                        }
                    }
                }
            }
            net.settle();
            if net.peers.len() > 1 {
                net.assert_converged();
            }
        }
    }

    #[test]
    fn name_clashes_are_settled_by_the_later_id() {
        let mut net = Net::new();
        net.join("a", Level::default(), 1.0);
        net.join("b", Level::default(), 2.0);
        net.peer("a").0.rename("owl".into());
        net.peer("b").0.rename("owl".into());
        net.connect();
        net.settle();
        assert_eq!(net.peer("a").0.settle_name_clash(), None);
        let renamed = net.peer("b").0.settle_name_clash().expect("b gives way");
        assert_ne!(renamed, "owl");
    }
}
