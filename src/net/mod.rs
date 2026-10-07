//! Editing a level together, in the way the checkers game plays over the network: peers meet in
//! a named room through a signaling server and then talk peer to peer over WebRTC (matchbox).
//! Every edit goes through one host that puts all edits in one order (see [`session`]), so
//! everyone in the room edits the same level, and sees each other's cursors and selections.
//!
//! Play mode runs on each machine for itself; the physics is not shared. While a peer plays,
//! world settings from the room apply live, and edits to elements wait until the game restarts
//! or the peer returns to the editor, since a running game refers to elements by position.

mod protocol;
mod room;
mod session;

use std::collections::HashMap;

use bevy::prelude::*;
use bevy::sprite::Anchor;
use bevy_matchbox::prelude::*;

use crate::editor::{Editor, History};
use crate::level::{Body, Level};
use crate::play::{CursorWorld, Mode, Restart};
use crate::visuals::WorldCamera;
use protocol::{NetMsg, Op, assign_ids, decode, encode};
pub use room::RoomId;
use room::{now_ms, peer_color, petname_avoiding, random_u64};
use session::{Remote, Session};

/// Introduces peers to each other; the edits themselves go peer to peer. Set `MATCHBOX_SERVER`
/// at compile time to use another one (natively also `DEMOLITION_SIGNALING` at run time). The
/// default is the checkers game's server; rooms here are prefixed so they never meet those.
const SIGNALING_SERVER: &str = match option_env!("MATCHBOX_SERVER") {
    Some(url) => url,
    None => "wss://omdurman-matchbox.fly.dev",
};

/// How often local edits are sent while they keep coming (a drag), and the cursor with them.
const SEND_INTERVAL: f32 = 0.05;
const CURSOR_INTERVAL: f32 = 0.1;
/// Cursor label size in screen pixels.
const LABEL_SIZE: f32 = 12.0;

pub struct NetPlugin;

impl Plugin for NetPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Net>().add_systems(
            Update,
            (
                maintain_socket,
                pump,
                send_cursor,
                (draw_cursors, sync_cursor_labels),
            )
                .chain()
                // After the editor and the panels changed the level this frame, and before a
                // restart spawns it.
                .after(crate::editor::EditSystems)
                .after(crate::ui::UiSystems)
                .after(crate::visuals::play_hotkeys)
                .before(crate::play::restart),
        );
    }
}

/// The room this peer is in and everything known about it.
#[derive(Resource)]
pub struct Net {
    /// The room to be in. Setting it (or clearing it) opens (or closes) the connection.
    pub room: Option<RoomId>,
    /// This player's name in the room.
    pub name: String,
    /// The room name being typed into the join field.
    pub draft: String,
    /// The connection's room; differs from `room` until the socket follows.
    socket_room: Option<RoomId>,
    session: Option<Session>,
    peer_ids: HashMap<String, PeerId>,
    cursors: HashMap<String, RemoteCursor>,
    /// Local edits not yet diffed (they are sent at most every [`SEND_INTERVAL`]).
    dirty: bool,
    since_send: f32,
    /// Element edits from the room that a running game hasn't taken in yet.
    stale: bool,
    /// What happened last, for the panel.
    pub status: String,
}

impl Default for Net {
    fn default() -> Self {
        Self {
            room: room::room_at_launch(),
            name: petname_avoiding(&[]),
            draft: String::new(),
            socket_room: None,
            session: None,
            peer_ids: HashMap::new(),
            cursors: HashMap::new(),
            dirty: false,
            since_send: 0.0,
            stale: false,
            status: String::new(),
        }
    }
}

impl Net {
    pub fn join(&mut self, room: RoomId) {
        room::show_room_in_url(Some(&room));
        self.status.clear();
        self.room = Some(room);
    }

    pub fn join_draft(&mut self) {
        match RoomId::parse(&self.draft) {
            Ok(room) => self.join(room),
            Err(e) => self.status = e,
        }
    }

    pub fn leave(&mut self) {
        room::show_room_in_url(None);
        self.status.clear();
        self.room = None;
    }

    pub fn invite_link(&self) -> Option<String> {
        self.room.as_ref().map(RoomId::invite_link)
    }

    /// One line on the room, for the panel.
    pub fn summary(&self) -> String {
        let Some(room) = &self.room else {
            return String::new();
        };
        let Some(session) = &self.session else {
            return format!("Connecting to room {}…", room.0);
        };
        let mut line = match session.peers().len() {
            0 => format!("Room {}: nobody else here yet", room.0),
            1 => format!("Room {}: you and 1 other", room.0),
            n => format!("Room {}: you and {n} others", room.0),
        };
        if !session.peers().is_empty() && !session.holds_room() {
            line += " (fetching the level…)";
        } else if self.stale {
            line += " (their edits wait until you restart or edit)";
        }
        if !self.status.is_empty() {
            line = format!("{line}\n{}", self.status);
        }
        line
    }

    /// Everyone in the room with their color, this peer first.
    pub fn people(&self) -> Vec<(String, Color)> {
        let Some(session) = &self.session else {
            return vec![];
        };
        let mut people = vec![(format!("{} (you)", self.name), peer_color(&session.me))];
        for peer in session.peers() {
            let name = session
                .names
                .get(peer)
                .cloned()
                .unwrap_or("joining…".into());
            people.push((name, peer_color(peer)));
        }
        people
    }
}

/// A peer's pointer as last reported, and where it is drawn (eased, so 10 Hz updates glide).
struct RemoteCursor {
    pos: Option<Vec2>,
    drawn: Vec2,
    selected: Option<u64>,
}

/// Opens the room's connection, or closes it when the room is left or changed.
fn maintain_socket(mut commands: Commands, mut net: ResMut<Net>) {
    if net.room == net.socket_room {
        return;
    }
    commands.remove_resource::<MatchboxSocket>();
    net.session = None;
    net.peer_ids.clear();
    net.cursors.clear();
    net.stale = false;
    net.socket_room = net.room.clone();
    if let Some(room) = &net.room {
        #[cfg(not(target_arch = "wasm32"))]
        let server = std::env::var("DEMOLITION_SIGNALING").unwrap_or(SIGNALING_SERVER.into());
        #[cfg(target_arch = "wasm32")]
        let server = SIGNALING_SERVER;
        let url = format!("{server}/demolition-{}", room.0);
        info!(%url, "joining room");
        commands.insert_resource(MatchboxSocket::from(
            WebRtcSocketBuilder::new(url)
                .reconnect_attempts(None)
                .add_reliable_channel(),
        ));
    }
}

/// Sends local edits, takes in the room's, and keeps the editor's view (selection, undo)
/// consistent with a level that changes under it.
fn pump(
    socket: Option<ResMut<MatchboxSocket>>,
    mut net: ResMut<Net>,
    mut level: ResMut<Level>,
    mut editor: ResMut<Editor>,
    mut history: ResMut<History>,
    mode: Res<State<Mode>>,
    restart: Res<Restart>,
    time: Res<Time<Real>>,
    mut typed: Local<(String, f32)>,
) {
    let Some(mut socket) = socket else { return };
    let net = &mut *net;
    if net.socket_room.is_none() {
        return;
    }
    if net.session.is_none() {
        let Some(id) = socket.id() else { return };
        // Elements need ids to be edited by several people. Undo steps from before the room
        // would come back without them, so the history starts over.
        if assign_ids(level.bypass_change_detection(), random_u64) {
            level.set_changed();
        }
        history.forget();
        net.session = Some(Session::new(
            id.to_string(),
            net.name.clone(),
            &level,
            now_ms(),
        ));
        info!(peer = %id, "in the room");
    }
    let session = net.session.as_mut().unwrap();

    let mut peers_changed = false;
    for (peer, state) in socket.update_peers() {
        peers_changed = true;
        match state {
            PeerState::Connected => {
                net.peer_ids.insert(peer.to_string(), peer);
            }
            PeerState::Disconnected => {
                net.peer_ids.remove(&peer.to_string());
                net.cursors.remove(&peer.to_string());
            }
        }
    }
    if peers_changed {
        session.set_peers(net.peer_ids.keys().cloned().collect());
    }
    // A new name goes out once it has stopped changing, not with every key typed.
    if typed.0 != net.name {
        *typed = (net.name.clone(), time.elapsed_secs());
    }
    if session.name != net.name && time.elapsed_secs() > typed.1 + 0.7 {
        session.rename(net.name.clone());
    }
    if let Some(name) = session.settle_name_clash() {
        net.status = format!(
            "Someone here is already called {}: you are {name} now",
            net.name
        );
        net.name = name;
    }
    session.greet();

    let inbox: Vec<(String, NetMsg)> = socket
        .channel_mut(0)
        .receive()
        .into_iter()
        .filter_map(|(from, bytes)| Some((from.to_string(), decode(&bytes)?)))
        .collect();

    // Local edits go out first: a level arriving from the room replaces what is on screen, so
    // nothing may be left unsent. Otherwise they are sent at most every SEND_INTERVAL.
    let playing = *mode.get() == Mode::Play;
    let catch_up = net.stale && (!playing || restart.0);
    let incoming = inbox
        .iter()
        .any(|(_, msg)| !matches!(msg, NetMsg::Cursor { .. }));
    net.dirty |= level.is_changed();
    net.since_send += time.delta_secs();
    if net.dirty && (net.since_send >= SEND_INTERVAL || incoming || catch_up) {
        net.dirty = false;
        net.since_send = 0.0;
        if assign_ids(level.bypass_change_detection(), random_u64) {
            level.set_changed();
        }
        session.local_edit(&level);
    }

    for (from, msg) in inbox {
        match msg {
            NetMsg::Cursor { pos, selected } => {
                let cursor = net.cursors.entry(from).or_insert(RemoteCursor {
                    pos,
                    drawn: pos.unwrap_or_default(),
                    selected,
                });
                cursor.pos = pos;
                cursor.selected = selected;
            }
            msg => session.receive(&from, msg),
        }
    }
    let remote = session.take_changes();

    if !remote.is_empty() || catch_up {
        let elements_changed = remote.iter().any(|r| match r {
            Remote::Reset => true,
            Remote::Ops(ops) => ops.iter().any(Op::touches_elements),
        });
        if playing && !restart.0 && (elements_changed || net.stale) {
            // The running game spawned the elements; only world settings apply live.
            net.stale = true;
            let settings: Vec<Op> = remote
                .iter()
                .filter_map(|r| match r {
                    Remote::Ops(ops) => Some(ops.iter().filter(|op| !op.touches_elements())),
                    Remote::Reset => None,
                })
                .flatten()
                .cloned()
                .collect();
            if !settings.is_empty() {
                protocol::apply(&mut level, &settings);
                protocol::apply(session.shown_mut(), &settings);
            }
        } else {
            let expected = session.expected();
            let reset = net.stale || remote.contains(&Remote::Reset);
            if reset {
                history.forget();
            } else {
                for r in &remote {
                    if let Remote::Ops(ops) = r {
                        history.rebase(|level| protocol::apply(level, ops));
                    }
                }
            }
            // Keep the selection on the same element as it moves in the list (or goes).
            let selected = editor
                .selected
                .and_then(|i| level.elements.get(i))
                .map(|e| e.id);
            let reselected =
                selected.and_then(|id| expected.elements.iter().position(|e| e.id == id));
            if editor.selected != reselected {
                editor.selected = reselected;
            }
            if *level != expected {
                *level = expected;
            }
            *session.shown_mut() = level.clone();
            net.stale = false;
        }
    }

    for (to, msg) in session.take_outbox() {
        send(&mut socket, &net.peer_ids, &to, &msg);
    }
}

fn send(socket: &mut MatchboxSocket, ids: &HashMap<String, PeerId>, to: &[String], msg: &NetMsg) {
    let Some(bytes) = encode(msg) else { return };
    for peer in to.iter().filter_map(|p| ids.get(p)) {
        if let Err(error) = socket.channel_mut(0).try_send(bytes.clone(), *peer) {
            warn!(%error, %peer, "send failed");
        }
    }
}

/// Tells the room where this pointer is and what it has selected, when that changes.
fn send_cursor(
    socket: Option<ResMut<MatchboxSocket>>,
    net: Res<Net>,
    cursor: Res<CursorWorld>,
    editor: Res<Editor>,
    level: Res<Level>,
    mode: Res<State<Mode>>,
    time: Res<Time<Real>>,
    mut last: Local<(f32, Option<(Option<Vec2>, Option<u64>)>)>,
) {
    let (Some(mut socket), Some(session)) = (socket, &net.session) else {
        return;
    };
    if session.peers().is_empty() || time.elapsed_secs() < last.0 + CURSOR_INTERVAL {
        return;
    }
    let selected = editor
        .selected
        .filter(|_| *mode.get() == Mode::Edit)
        .and_then(|i| level.elements.get(i))
        .map(|e| e.id);
    let now = (cursor.0.map(|p| p.round()), selected);
    if last.1 == Some(now) {
        return;
    }
    *last = (time.elapsed_secs(), Some(now));
    let peers: Vec<String> = session.peers().to_vec();
    let msg = NetMsg::Cursor {
        pos: now.0,
        selected: now.1,
    };
    send(&mut socket, &net.peer_ids, &peers, &msg);
}

/// Other people's pointers, and outlines around the elements they have selected.
fn draw_cursors(
    mut gizmos: Gizmos,
    mut net: ResMut<Net>,
    level: Res<Level>,
    mode: Res<State<Mode>>,
    cameras: Query<&Projection, With<WorldCamera>>,
    time: Res<Time<Real>>,
) {
    let scale = match cameras.single() {
        Ok(Projection::Orthographic(ortho)) => ortho.scale,
        _ => 1.0,
    };
    let ease = 1.0 - (-12.0 * time.delta_secs()).exp();
    for (peer, cursor) in net.bypass_change_detection().cursors.iter_mut() {
        let color = peer_color(peer);
        if *mode.get() == Mode::Edit
            && let Some(element) = cursor
                .selected
                .and_then(|id| level.elements.iter().find(|e| e.id == id))
        {
            let size = element.at_resolution(level.resolution).size() + Vec2::splat(10.0 * scale);
            if element.is_round() {
                gizmos.circle_2d(element.pos, size.x / 2.0, color);
            } else {
                gizmos.rect_2d(element.pose(), size, color);
            }
            if let Body::Ball { .. } = element.body {
                // A ball's outline is easy to miss against its own edge.
                gizmos.circle_2d(element.pos, size.x / 2.0 + 3.0 * scale, color);
            }
        }
        let Some(pos) = cursor.pos else { continue };
        cursor.drawn = cursor.drawn.lerp(pos, ease);
        let tip = cursor.drawn;
        // An arrow pointer, the same size on screen at any zoom.
        let s = 14.0 * scale;
        gizmos.linestrip_2d(
            [
                tip,
                tip + Vec2::new(0.0, -s),
                tip + Vec2::new(0.3, -0.75) * s,
                tip + Vec2::new(0.7, -0.7) * s,
                tip,
            ],
            color,
        );
    }
}

/// The name label beside each remote pointer.
#[derive(Component)]
struct CursorLabel(String);

fn sync_cursor_labels(
    mut commands: Commands,
    net: Res<Net>,
    cameras: Query<&Projection, With<WorldCamera>>,
    mut labels: Query<(
        Entity,
        &CursorLabel,
        &mut Transform,
        &mut Text2d,
        &mut Visibility,
    )>,
) {
    let scale = match cameras.single() {
        Ok(Projection::Orthographic(ortho)) => ortho.scale,
        _ => 1.0,
    };
    let names = net.session.as_ref().map(|s| &s.names);
    for (entity, label, mut transform, mut text, mut visibility) in &mut labels {
        let Some(cursor) = net.cursors.get(&label.0) else {
            commands.entity(entity).despawn();
            continue;
        };
        let name = names
            .and_then(|n| n.get(&label.0))
            .cloned()
            .unwrap_or_default();
        if text.0 != name {
            text.0 = name;
        }
        let at = (cursor.drawn + Vec2::new(12.0, -16.0) * scale).extend(20.0);
        if transform.translation != at || transform.scale.x != scale {
            *transform = Transform::from_translation(at).with_scale(Vec3::splat(scale));
        }
        visibility.set_if_neq(if cursor.pos.is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
    for peer in net.cursors.keys() {
        if !labels.iter().any(|(_, label, ..)| label.0 == *peer) {
            commands.spawn((
                CursorLabel(peer.clone()),
                Text2d::new(""),
                TextFont {
                    font_size: FontSize::Px(LABEL_SIZE),
                    ..default()
                },
                TextColor(peer_color(peer)),
                Anchor::TOP_LEFT,
                Transform::default(),
                Visibility::Hidden,
            ));
        }
    }
}
