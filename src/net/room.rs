//! Room names and where they come from, pet names, and the randomness and clock the room needs.
//!
//! On the web the room travels in the URL fragment (`#room=name`), so the address bar is the
//! invite: send the link and the recipient lands in the same room. The fragment, unlike a query
//! parameter, never reaches a server. Natively the room comes from `--room <name>`.

use bevy::prelude::*;

/// The room to edit in. Peers in the same room find each other through the signaling server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomId(pub String);

impl RoomId {
    pub const MAX_LEN: usize = 40;

    /// Room names go into the signaling URL's path, so they are limited to characters that need
    /// no escaping, and to ASCII, so two people can agree on one by reading it out.
    pub fn parse(name: &str) -> Result<Self, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("a room name cannot be empty".into());
        }
        if name.chars().count() > Self::MAX_LEN {
            return Err(format!(
                "a room name may be at most {} characters",
                Self::MAX_LEN
            ));
        }
        if let Some(c) = name
            .chars()
            .find(|c| !(c.is_ascii_alphanumeric() || *c == '-' || *c == '_'))
        {
            return Err(format!(
                "'{c}' is not allowed in a room name; use letters, digits, '-' or '_'"
            ));
        }
        Ok(Self(name.to_string()))
    }

    /// A fresh room: five unambiguous characters.
    pub fn random() -> Self {
        const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
        let mut seed = random_u64();
        let mut id = String::new();
        for _ in 0..5 {
            id.push(ALPHABET[(seed % ALPHABET.len() as u64) as usize] as char);
            seed /= ALPHABET.len() as u64;
        }
        Self(id)
    }

    /// A link that opens this room in the browser.
    pub fn invite_link(&self) -> String {
        format!("{}#room={}", crate::level::share_base(), self.0)
    }
}

/// The room named at launch: the URL fragment on the web, `--room` natively. An invalid name is
/// ignored rather than reported, so a damaged link opens the editor alone.
pub fn room_at_launch() -> Option<RoomId> {
    #[cfg(target_arch = "wasm32")]
    let source = web_sys::window().and_then(|w| w.location().hash().ok());
    #[cfg(not(target_arch = "wasm32"))]
    let source = {
        let args: Vec<String> = std::env::args().collect();
        args.iter()
            .position(|a| a == "--room")
            .and_then(|i| args.get(i + 1))
            .map(|name| format!("room={name}"))
    };
    room_from_fragment(&source?)
}

fn room_from_fragment(fragment: &str) -> Option<RoomId> {
    fragment
        .trim_start_matches('#')
        .split('&')
        .find_map(|pair| pair.strip_prefix("room="))
        .and_then(|name| RoomId::parse(name).ok())
}

/// Puts the room into the address bar (or takes it out), so the page's URL is always the invite
/// to where this peer is. Replaces the history entry instead of adding one. No-op natively.
pub fn show_room_in_url(room: Option<&RoomId>) {
    #[cfg(target_arch = "wasm32")]
    {
        let Some(window) = web_sys::window() else {
            return;
        };
        let location = window.location();
        let (Ok(path), Ok(search)) = (location.pathname(), location.search()) else {
            return;
        };
        let url = match room {
            Some(room) => format!("{path}{search}#room={}", room.0),
            None => format!("{path}{search}"),
        };
        if let Ok(history) = window.history() {
            let _ = history.replace_state_with_url(
                &web_sys::wasm_bindgen::JsValue::NULL,
                "",
                Some(&url),
            );
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = room;
}

/// Short names a player starts with: one word, easy to say across the table.
const PET_NAMES: &[&str] = &[
    "otter", "falcon", "maple", "ember", "comet", "panda", "lynx", "heron", "quail", "gecko",
    "koala", "raven", "tiger", "bison", "crane", "dingo", "eagle", "ibex", "orca", "yak", "hare",
    "moth", "wren", "toad", "newt", "elk", "fox", "owl", "bee", "finch", "mole", "puffin",
    "badger", "marten", "vole",
];

/// A pet name nobody in `taken` goes by (numbered once all are taken).
pub fn petname_avoiding(taken: &[&str]) -> String {
    let free: Vec<&str> = PET_NAMES
        .iter()
        .copied()
        .filter(|name| !taken.contains(name))
        .collect();
    let pick = |names: &[&str]| names[(random_u64() % names.len() as u64) as usize].to_string();
    if free.is_empty() {
        format!("{}-{}", pick(PET_NAMES), random_u64() % 1000)
    } else {
        pick(&free)
    }
}

/// A random number that differs between processes and calls. std has no randomness on
/// wasm32-unknown-unknown, so the browser's `Math.random()` provides it there.
pub fn random_u64() -> u64 {
    #[cfg(target_arch = "wasm32")]
    {
        let draw = || (js_sys::Math::random() * (1u64 << 32) as f64) as u64;
        draw() << 32 | draw()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::hash::{BuildHasher, Hasher};
        use std::sync::atomic::{AtomicU64, Ordering};
        static CALLS: AtomicU64 = AtomicU64::new(0);
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u64(CALLS.fetch_add(1, Ordering::Relaxed));
        hasher.finish()
    }
}

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> f64 {
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Date::now()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |d| d.as_secs_f64() * 1000.0)
    }
}

/// A color for a peer, the same on every machine: its cursor and selection wear it.
pub fn peer_color(peer: &str) -> Color {
    let hash = peer.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100_0000_01b3)
    });
    Color::hsl((hash % 360) as f32, 0.8, 0.62)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rooms_come_from_the_fragment() {
        assert_eq!(room_from_fragment("#room=abc").unwrap().0, "abc");
        assert_eq!(room_from_fragment("x=1&room=a-b_2").unwrap().0, "a-b_2");
        assert_eq!(room_from_fragment("#room="), None);
        assert_eq!(room_from_fragment("#room=a/b"), None);
        assert_eq!(room_from_fragment(""), None);
    }

    #[test]
    fn room_names_are_checked() {
        assert!(RoomId::parse(" ok-room ").is_ok());
        assert!(RoomId::parse("café").is_err());
        assert!(RoomId::parse(&"a".repeat(41)).is_err());
        let random = RoomId::random();
        assert_eq!(random.0.len(), 5);
        assert_ne!(random, RoomId::random());
    }

    #[test]
    fn a_clash_draws_a_free_name() {
        let taken: Vec<&str> = PET_NAMES.iter().copied().filter(|n| *n != "owl").collect();
        assert_eq!(petname_avoiding(&taken), "owl");
        assert!(!PET_NAMES.contains(&petname_avoiding(PET_NAMES).as_str()));
    }
}
