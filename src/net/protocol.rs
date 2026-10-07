//! What travels between the peers of a room: edits to the level as operations, and the
//! messages that carry them.
//!
//! An edit is never sent as a whole level. Two people working on different elements at the
//! same time would otherwise overwrite each other's work with their stale copy of the rest.
//! Instead each peer describes what it changed — this element now looks like this, that one is
//! gone, gravity is now 400 — and only what it changed. Operations are idempotent and carry
//! whole values, so applying one twice, or after a newer state arrived, does no harm.

use serde::{Deserialize, Serialize};

use crate::level::{Element, Level};
use crate::materials::{MaterialKind, MaterialParams, Strength};

/// One change to a level.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Op {
    /// Add the element, or replace the one with its id. A new element goes right after
    /// `after` (to the front for `None`, to the end if `after` is gone), where its author put it.
    Put {
        element: Element,
        after: Option<u64>,
    },
    Remove(u64),
    /// Reorder the elements to this order of ids; elements not listed keep their order at the end.
    Order(Vec<u64>),
    Set(Setting),
}

/// One world setting of a level, so that concurrent edits of different settings both survive.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Setting {
    Name(String),
    Gravity(f32),
    Substeps(u32),
    Resolution(f32),
    View(glam::Vec2),
    Bounds(glam::Vec2),
    Gun(bool),
    GunPos(glam::Vec2),
    Material(MaterialKind, MaterialParams),
    Pins(Strength),
}

impl Op {
    /// Whether this changes the elements, which a running game has already spawned.
    pub fn touches_elements(&self) -> bool {
        !matches!(self, Op::Set(_))
    }
}

/// Applies operations in order.
pub fn apply(level: &mut Level, ops: &[Op]) {
    for op in ops {
        apply_one(level, op);
    }
}

fn apply_one(level: &mut Level, op: &Op) {
    let elements = &mut level.elements;
    match op {
        Op::Put { element, after } => {
            if let Some(existing) = elements.iter_mut().find(|e| e.id == element.id) {
                *existing = element.clone();
            } else {
                let at = match after {
                    None => 0,
                    Some(after) => elements
                        .iter()
                        .position(|e| e.id == *after)
                        .map_or(elements.len(), |i| i + 1),
                };
                elements.insert(at, element.clone());
            }
        }
        Op::Remove(id) => elements.retain(|e| e.id != *id),
        Op::Order(ids) => {
            // Stable, so unlisted elements keep their relative order behind the listed ones.
            elements.sort_by_key(|e| ids.iter().position(|id| *id == e.id).unwrap_or(usize::MAX));
        }
        Op::Set(setting) => match setting.clone() {
            Setting::Name(name) => level.name = name,
            Setting::Gravity(g) => level.gravity = g,
            Setting::Substeps(n) => level.substeps = n,
            Setting::Resolution(r) => level.resolution = r,
            Setting::View(v) => level.view = v,
            Setting::Bounds(b) => level.bounds = b,
            Setting::Gun(on) => level.gun = on,
            Setting::GunPos(p) => level.gun_pos = p,
            Setting::Material(kind, params) => level.materials.table[kind as usize] = params,
            Setting::Pins(strength) => level.materials.pins = strength,
        },
    }
}

/// The operations that turn `from` into `to`. Elements are matched by id, so every element of
/// both levels needs a unique one (see [`assign_ids`]).
pub fn diff(from: &Level, to: &Level) -> Vec<Op> {
    // Destructured, so a new level field fails to compile here until it is synchronised.
    let Level {
        name,
        gravity,
        substeps,
        resolution,
        view,
        bounds,
        gun,
        gun_pos,
        elements,
        materials,
    } = to;
    let mut ops = Vec::new();
    let mut set = |changed: bool, setting: Setting| {
        if changed {
            ops.push(Op::Set(setting));
        }
    };
    set(from.name != *name, Setting::Name(name.clone()));
    set(from.gravity != *gravity, Setting::Gravity(*gravity));
    set(from.substeps != *substeps, Setting::Substeps(*substeps));
    set(
        from.resolution != *resolution,
        Setting::Resolution(*resolution),
    );
    set(from.view != *view, Setting::View(*view));
    set(from.bounds != *bounds, Setting::Bounds(*bounds));
    set(from.gun != *gun, Setting::Gun(*gun));
    set(from.gun_pos != *gun_pos, Setting::GunPos(*gun_pos));
    for kind in MaterialKind::ALL {
        let params = &materials.table[kind as usize];
        set(
            from.materials.table[kind as usize] != *params,
            Setting::Material(kind, params.clone()),
        );
    }
    set(
        from.materials.pins != materials.pins,
        Setting::Pins(materials.pins),
    );

    for old in &from.elements {
        if !elements.iter().any(|e| e.id == old.id) {
            ops.push(Op::Remove(old.id));
        }
    }
    for (i, element) in elements.iter().enumerate() {
        if from.elements.iter().find(|e| e.id == element.id) == Some(element) {
            continue;
        }
        ops.push(Op::Put {
            element: element.clone(),
            after: i.checked_sub(1).map(|p| elements[p].id),
        });
    }
    // Puts keep existing elements where they are, so a reordering needs saying.
    let mut result = from.clone();
    apply(&mut result, &ops);
    if result
        .elements
        .iter()
        .map(|e| e.id)
        .ne(elements.iter().map(|e| e.id))
    {
        ops.push(Op::Order(elements.iter().map(|e| e.id).collect()));
    }
    ops
}

/// Gives every element without an id, or with one an earlier element already has (a fresh
/// duplicate), a new random one. Returns whether anything changed.
pub fn assign_ids(level: &mut Level, mut fresh: impl FnMut() -> u64) -> bool {
    let mut seen = std::collections::HashSet::new();
    let mut changed = false;
    for element in &mut level.elements {
        while element.id == 0 || !seen.insert(element.id) {
            element.id = fresh();
            changed = true;
        }
    }
    changed
}

/// A message between peers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum NetMsg {
    /// Introduces the sender to a peer that just connected (again after a rename). It carries
    /// what the host needs to decide whose level the room keeps when two sides meet.
    Hello {
        name: String,
        /// The sender has been in the room together with someone: its level is the room's.
        holds_room: bool,
        /// When the sender entered the room (ms since the Unix epoch): of two peers that both
        /// edited alone, the one that was there first keeps its level.
        since: f64,
        /// Whom the sender takes for the host. A host settles a newcomer's level only from a
        /// Hello addressed to it as the host, which is recent: peers greet a new host again.
        host: String,
        level: Level,
    },
    /// Guest → host: edits for the host to put in order. Never applied as they arrive.
    Submit { nonce: u32, ops: Vec<Op> },
    /// Host → everyone: edits in their one agreed order (the order the host sends them in, on
    /// a reliable, ordered channel). Only these are applied, so every peer ends up with the same
    /// level. `origin` and `nonce` let the author recognise its own.
    Sequenced {
        origin: String,
        nonce: u32,
        ops: Vec<Op>,
    },
    /// Host → a newcomer, or everyone when the room takes a newcomer's level or gets a new
    /// host: the whole level. `origin` is the peer whose level it was.
    Snapshot {
        origin: String,
        level: Level,
        /// The newest edit of each peer the level contains, so a peer stops waiting for the
        /// host to pass on edits it already has.
        passed: Vec<(String, u32)>,
    },
    /// Where the sender's pointer is in the world, and which element it has selected.
    Cursor {
        pos: Option<glam::Vec2>,
        selected: Option<u64>,
    },
}

/// Messages are RON, like saved levels and share links; long ones (levels) are deflated.
/// The first byte says which.
pub fn encode(msg: &NetMsg) -> Option<Box<[u8]>> {
    let text = ron::to_string(msg)
        .inspect_err(|e| bevy::log::error!("could not encode a message: {e}"))
        .ok()?;
    let mut bytes = if text.len() > 256 {
        let mut packed = vec![1];
        packed.extend(miniz_oxide::deflate::compress_to_vec(text.as_bytes(), 6));
        packed
    } else {
        let mut raw = vec![0];
        raw.extend(text.as_bytes());
        raw
    };
    bytes.shrink_to_fit();
    Some(bytes.into_boxed_slice())
}

pub fn decode(bytes: &[u8]) -> Option<NetMsg> {
    let (&kind, body) = bytes.split_first()?;
    let text = match kind {
        0 => body.to_vec(),
        1 => miniz_oxide::inflate::decompress_to_vec_with_limit(body, 16 << 20).ok()?,
        _ => return None,
    };
    ron::from_str(std::str::from_utf8(&text).ok()?)
        .inspect_err(|e| bevy::log::warn!("could not decode a message: {e}"))
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::{Body, preset_lab, preset_pong};
    use glam::Vec2;

    fn ids(level: &mut Level) {
        let mut next = 0;
        assign_ids(level, || {
            next += 1;
            next
        });
    }

    fn with_ids(mut level: Level) -> Level {
        ids(&mut level);
        level
    }

    fn check(from: &Level, to: &Level) -> Vec<Op> {
        let ops = diff(from, to);
        let mut result = from.clone();
        apply(&mut result, &ops);
        assert_eq!(&result, to, "applying the diff must give the target");
        ops
    }

    #[test]
    fn nothing_changed_is_no_ops() {
        let level = with_ids(preset_lab());
        assert!(diff(&level, &level).is_empty());
    }

    #[test]
    fn one_moved_element_is_one_put() {
        let from = with_ids(preset_lab());
        let mut to = from.clone();
        to.elements[3].pos += Vec2::X;
        let ops = check(&from, &to);
        assert_eq!(ops.len(), 1, "{ops:?}");
    }

    #[test]
    fn adding_removing_reordering_and_settings_all_round_trip() {
        let from = with_ids(preset_lab());
        let mut to = from.clone();
        to.elements.remove(2);
        let copy = Element {
            id: 999,
            ..to.elements[0].clone()
        };
        to.elements.insert(1, copy);
        to.elements.swap(3, 5);
        to.gravity = -100.0;
        to.materials.table[1].density *= 2.0;
        to.materials.pins.break_strain = 0.5;
        check(&from, &to);
        // And a whole new level, as when a preset is loaded.
        check(&from, &with_ids(preset_pong()));
        check(&with_ids(preset_pong()), &Level::default());
    }

    #[test]
    fn concurrent_edits_of_different_elements_both_survive() {
        let base = with_ids(preset_lab());
        let mut a = base.clone();
        a.elements[0].pos = Vec2::new(1.0, 2.0);
        let mut b = base.clone();
        b.elements[1].angle = 1.0;
        b.gravity = 5.0;
        // The host orders a's edit first, then b's: both land.
        let mut room = base.clone();
        apply(&mut room, &diff(&base, &a));
        apply(&mut room, &diff(&base, &b));
        assert_eq!(room.elements[0].pos, Vec2::new(1.0, 2.0));
        assert_eq!(room.elements[1].angle, 1.0);
        assert_eq!(room.gravity, 5.0);
    }

    #[test]
    fn a_put_after_a_removed_element_goes_to_the_end() {
        let mut level = with_ids(preset_lab());
        let n = level.elements.len();
        let element = Element {
            id: 4242,
            body: Body::Ball {
                radius: 3.0,
                density: 1.0,
                restitution: 0.0,
                friction: 0.0,
                color: [1.0; 3],
            },
            ..Default::default()
        };
        apply(
            &mut level,
            &[Op::Put {
                element,
                after: Some(123_456),
            }],
        );
        assert_eq!(level.elements.len(), n + 1);
        assert_eq!(level.elements[n].id, 4242);
    }

    #[test]
    fn duplicates_get_fresh_ids() {
        let mut level = with_ids(preset_lab());
        let copy = level.elements[0].clone();
        level.elements.push(copy);
        let mut next = 1000;
        assert!(assign_ids(&mut level, || {
            next += 1;
            next
        }));
        let mut seen: Vec<u64> = level.elements.iter().map(|e| e.id).collect();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), level.elements.len());
        assert_eq!(
            level.elements.last().unwrap().id,
            1001,
            "the copy is renamed"
        );
    }

    #[test]
    fn messages_round_trip_exactly() {
        let level = with_ids(preset_pong());
        let msgs = [
            NetMsg::Hello {
                name: "otter".into(),
                holds_room: true,
                since: 1.7e12,
                host: "otter".into(),
                level: level.clone(),
            },
            NetMsg::Submit {
                nonce: 1,
                ops: diff(&Level::default(), &level),
            },
            NetMsg::Cursor {
                pos: Some(Vec2::new(0.1, -3.3)),
                selected: None,
            },
        ];
        for msg in msgs {
            let bytes = encode(&msg).unwrap();
            assert_eq!(decode(&bytes).unwrap(), msg);
        }
        assert_eq!(decode(&[7, 1, 2]), None);
        assert_eq!(decode(&[]), None);
    }
}
