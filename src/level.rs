//! Levels are plain data: a list of elements plus world settings. The editor edits a `Level`,
//! Play mode spawns it, and levels round-trip through RON files in `levels/`.

use std::collections::HashMap;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::materials::{MaterialKind, Materials};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Control {
    #[default]
    None,
    /// Follows the mouse cursor.
    Cursor,
    Arrows,
    Wasd,
}

impl Control {
    pub const ALL: [Control; 4] = [
        Control::None,
        Control::Cursor,
        Control::Arrows,
        Control::Wasd,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Control::None => "None",
            Control::Cursor => "Cursor",
            Control::Arrows => "Arrow keys",
            Control::Wasd => "WASD",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Axis {
    #[default]
    Free,
    Horizontal,
    Vertical,
}

impl Axis {
    pub const ALL: [Axis; 3] = [Axis::Free, Axis::Horizontal, Axis::Vertical];

    pub fn name(self) -> &'static str {
        match self {
            Axis::Free => "Free",
            Axis::Horizontal => "Horizontal only",
            Axis::Vertical => "Vertical only",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Player {
    #[default]
    One,
    Two,
}

impl Player {
    pub fn index(self) -> usize {
        self as usize
    }

    pub fn name(self) -> &'static str {
        match self {
            Player::One => "P1",
            Player::Two => "P2",
        }
    }
}

/// Who receives an element's points when it is destroyed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Credit {
    /// Whoever last hit it: the owner of the paddle, ball or shot that touched it last
    /// (ownership passes on through hits). Nobody, if nothing owned has touched it.
    #[default]
    LastHitter,
    Player(Player),
}

impl Credit {
    pub const ALL: [Credit; 3] = [
        Credit::LastHitter,
        Credit::Player(Player::One),
        Credit::Player(Player::Two),
    ];

    pub fn name(self) -> &'static str {
        match self {
            Credit::LastHitter => "last hitter",
            Credit::Player(p) => p.name(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Body {
    /// A destructible grid of cells.
    Lattice {
        material: MaterialKind,
        cols: i32,
        rows: i32,
        cell: f32,
        round: bool,
    },
    /// A solid, indestructible ball.
    Ball {
        radius: f32,
        density: f32,
        restitution: f32,
        friction: f32,
        color: [f32; 3],
    },
    /// An indestructible static block (kinematic when controlled).
    Wall {
        width: f32,
        height: f32,
        color: [f32; 3],
    },
}

impl Body {
    pub fn kind_name(&self) -> &'static str {
        match self {
            Body::Lattice { .. } => "Lattice",
            Body::Ball { .. } => "Ball",
            Body::Wall { .. } => "Wall",
        }
    }
}

/// Which cells are pinned to the anchor. For balls, any flag pins the center.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pins {
    pub left: bool,
    pub right: bool,
    pub top: bool,
    pub bottom: bool,
    pub center: bool,
    /// Pin every n-th cell along a side (the last one is always pinned).
    pub every: u32,
}

impl Default for Pins {
    fn default() -> Self {
        Self {
            left: false,
            right: false,
            top: false,
            bottom: false,
            center: false,
            every: 1,
        }
    }
}

impl Pins {
    pub fn any(&self) -> bool {
        self.left || self.right || self.top || self.bottom || self.center
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Element {
    pub name: String,
    pub pos: Vec2,
    /// Radians.
    pub angle: f32,
    pub velocity: Vec2,
    pub body: Body,
    pub pins: Pins,
    /// Controlled elements hang from a kinematic carrier by their pins (the center if none).
    pub control: Control,
    pub axis: Axis,
    pub speed: f32,
    /// How far a controlled element may move from its start position (0 = anywhere in bounds).
    /// Carriers are kinematic and push through anything, so this is what keeps a paddle
    /// out of the walls.
    pub range: f32,
    /// The player this element belongs to. Whatever it hits is tagged as hit by that player,
    /// and the tag passes on (paddle → ball → target).
    pub owner: Option<Player>,
    /// Awarded once the element counts as destroyed.
    pub points: i32,
    pub credit: Credit,
    /// Fraction of internal bonds lost at which a lattice counts as destroyed. Losing all its
    /// pins (knocked loose) or leaving the level bounds counts as destroyed too.
    pub destroyed_at: f32,
    /// Steer the element's velocity toward this speed (0 = off). Keeps a Pong ball going.
    pub keep_speed: f32,
    /// Added to the keep-speed target per second the element has been alive (escalation).
    pub speed_ramp: f32,
    /// Spawn a fresh copy after this element is destroyed.
    pub respawn: bool,
    /// Restitution override that beats whatever the element hits.
    pub bounce: Option<f32>,
    /// Friction override that beats whatever the element touches (0 = frictionless).
    pub friction: Option<f32>,
}

impl Default for Element {
    fn default() -> Self {
        Self {
            name: String::new(),
            pos: Vec2::ZERO,
            angle: 0.0,
            velocity: Vec2::ZERO,
            body: Body::Lattice {
                material: MaterialKind::Wood,
                cols: 8,
                rows: 8,
                cell: 10.0,
                round: false,
            },
            pins: Pins::default(),
            control: Control::None,
            axis: Axis::Free,
            speed: 600.0,
            range: 0.0,
            owner: None,
            points: 0,
            credit: Credit::LastHitter,
            destroyed_at: 0.5,
            keep_speed: 0.0,
            speed_ramp: 0.0,
            respawn: false,
            bounce: None,
            friction: None,
        }
    }
}

impl Element {
    /// Unrotated size of the element's footprint.
    pub fn size(&self) -> Vec2 {
        match &self.body {
            Body::Lattice {
                cols, rows, cell, ..
            } => Vec2::new(*cols as f32, *rows as f32) * *cell,
            Body::Ball { radius, .. } => Vec2::splat(radius * 2.0),
            Body::Wall { width, height, .. } => Vec2::new(*width, *height),
        }
    }

    pub fn is_round(&self) -> bool {
        matches!(
            self.body,
            Body::Ball { .. } | Body::Lattice { round: true, .. }
        )
    }

    pub fn pose(&self) -> Isometry2d {
        Isometry2d::new(self.pos, Rot2::radians(self.angle))
    }

    pub fn contains(&self, point: Vec2) -> bool {
        let local = self.pose().inverse() * point;
        let half = self.size() * 0.5;
        if self.is_round() {
            (local / half).length_squared() <= 1.0
        } else {
            local.x.abs() <= half.x && local.y.abs() <= half.y
        }
    }

    /// Pins as (col, row, offset within the cell), for lattices.
    pub fn lattice_pins(&self, has_cell: impl Fn(i32, i32) -> bool) -> Vec<(i32, i32, Vec2)> {
        let Body::Lattice {
            cols, rows, cell, ..
        } = self.body
        else {
            return vec![];
        };
        let h = cell * 0.5;
        let every = self.pins.every.max(1) as i32;
        let mut pins = vec![];
        // Walks along a side and pins the outermost present cell of each line.
        let mut side = |n: i32, cell_at: &dyn Fn(i32) -> Option<(i32, i32)>, offset: Vec2| {
            for k in 0..n {
                if k % every != 0 && k != n - 1 {
                    continue;
                }
                if let Some((i, j)) = cell_at(k) {
                    pins.push((i, j, offset));
                }
            }
        };
        if self.pins.left {
            side(
                rows,
                &|j| (0..cols).find(|&i| has_cell(i, j)).map(|i| (i, j)),
                Vec2::new(-h, 0.0),
            );
        }
        if self.pins.right {
            side(
                rows,
                &|j| (0..cols).rev().find(|&i| has_cell(i, j)).map(|i| (i, j)),
                Vec2::new(h, 0.0),
            );
        }
        if self.pins.bottom {
            side(
                cols,
                &|i| (0..rows).find(|&j| has_cell(i, j)).map(|j| (i, j)),
                Vec2::new(0.0, -h),
            );
        }
        if self.pins.top {
            side(
                cols,
                &|i| (0..rows).rev().find(|&j| has_cell(i, j)).map(|j| (i, j)),
                Vec2::new(0.0, h),
            );
        }
        let wants_center = self.pins.center || (self.control != Control::None && !self.pins.any());
        if wants_center {
            pins.push((cols / 2, rows / 2, Vec2::ZERO));
        }
        pins
    }
}

#[derive(Resource, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Level {
    pub name: String,
    /// Downward acceleration in px/s².
    pub gravity: f32,
    pub substeps: u32,
    /// Minimum visible world area.
    pub view: Vec2,
    /// Half extents of the playable area; anything leaving it is removed.
    pub bounds: Vec2,
    pub gun: bool,
    pub gun_pos: Vec2,
    pub elements: Vec<Element>,
    /// Material behaviour for this level.
    pub materials: Materials,
}

impl Default for Level {
    fn default() -> Self {
        Self {
            name: "untitled".into(),
            gravity: 900.0,
            substeps: 32,
            view: Vec2::new(1700.0, 950.0),
            bounds: Vec2::new(3000.0, 2000.0),
            gun: true,
            gun_pos: Vec2::new(-700.0, -330.0),
            elements: vec![],
            materials: Materials::default(),
        }
    }
}

fn lattice(material: MaterialKind, cols: i32, rows: i32, cell: f32) -> Body {
    Body::Lattice {
        material,
        cols,
        rows,
        cell,
        round: false,
    }
}

fn wall(name: &str, pos: Vec2, width: f32, height: f32) -> Element {
    Element {
        name: name.into(),
        pos,
        body: Body::Wall {
            width,
            height,
            color: [0.22, 0.22, 0.26],
        },
        ..default()
    }
}

pub fn preset_empty() -> Level {
    Level {
        elements: vec![wall("Floor", Vec2::new(0.0, -600.0), 6000.0, 400.0)],
        ..default()
    }
}

/// The original sandbox: four pinned structures under gravity, and a gun.
pub fn preset_lab() -> Level {
    let pins = |f: fn(&mut Pins)| {
        let mut p = Pins::default();
        f(&mut p);
        p
    };
    Level {
        name: "lab".into(),
        elements: vec![
            wall("Floor", Vec2::new(0.0, -600.0), 6000.0, 400.0),
            Element {
                name: "Pillar".into(),
                destroyed_at: 0.04,
                points: 100,
                pos: Vec2::new(-245.0, -229.5),
                body: lattice(MaterialKind::Concrete, 4, 34, 10.0),
                pins: pins(|p| p.bottom = true),
                ..default()
            },
            wall("Cantilever mount", Vec2::new(-23.0, 130.0), 34.0, 140.0),
            Element {
                name: "Cantilever".into(),
                destroyed_at: 0.04,
                points: 200,
                pos: Vec2::new(115.0, 130.0),
                body: lattice(MaterialKind::Steel, 24, 3, 10.0),
                pins: pins(|p| p.left = true),
                ..default()
            },
            wall("Ceiling", Vec2::new(440.0, 344.0), 180.0, 16.0),
            Element {
                name: "Hanging plate".into(),
                destroyed_at: 0.04,
                points: 150,
                pos: Vec2::new(445.0, 265.0),
                body: lattice(MaterialKind::Glass, 14, 14, 10.0),
                pins: pins(|p| {
                    p.top = true;
                    p.every = 13;
                }),
                ..default()
            },
            wall("Bridge pier L", Vec2::new(139.0, -270.0), 30.0, 260.0),
            wall("Bridge pier R", Vec2::new(611.0, -270.0), 30.0, 260.0),
            Element {
                name: "Bridge".into(),
                destroyed_at: 0.04,
                points: 150,
                pos: Vec2::new(375.0, -155.0),
                body: lattice(MaterialKind::Wood, 44, 2, 10.0),
                pins: pins(|p| {
                    p.left = true;
                    p.right = true;
                }),
                ..default()
            },
        ],
        ..default()
    }
}

/// Two-player Pong in zero gravity: smash the glass goal blocks behind your opponent's paddle.
pub fn preset_pong() -> Level {
    let mut elements = vec![
        wall("Back wall L", Vec2::new(-890.0, 0.0), 20.0, 1000.0),
        wall("Back wall R", Vec2::new(890.0, 0.0), 20.0, 1000.0),
        wall("Outer top", Vec2::new(0.0, 490.0), 1800.0, 20.0),
        wall("Outer bottom", Vec2::new(0.0, -490.0), 1800.0, 20.0),
    ];
    for (name, y, top) in [("Top wall", 465.0, true), ("Bottom wall", -465.0, false)] {
        elements.push(Element {
            name: name.into(),
            pos: Vec2::new(0.0, y),
            body: lattice(MaterialKind::Concrete, 85, 1, 20.0),
            pins: Pins {
                top,
                bottom: !top,
                every: 3,
                ..default()
            },
            ..default()
        });
    }
    for (side, x, player, control) in [
        ("L", -760.0, Player::One, Control::Wasd),
        ("R", 760.0, Player::Two, Control::Arrows),
    ] {
        elements.push(Element {
            name: format!("Paddle {side}"),
            pos: Vec2::new(x, 0.0),
            body: lattice(MaterialKind::Steel, 2, 10, 12.0),
            pins: Pins {
                left: x < 0.0,
                right: x > 0.0,
                every: 3,
                ..default()
            },
            control,
            owner: Some(player),
            axis: Axis::Vertical,
            speed: 650.0,
            range: 360.0,
            ..default()
        });
        // Goals behind this paddle score for the other player.
        let scorer = if player == Player::One {
            Player::Two
        } else {
            Player::One
        };
        for k in 0..5 {
            elements.push(Element {
                name: format!("Goal {side}{}", k + 1),
                pos: Vec2::new(x.signum() * 835.0, -336.0 + 168.0 * k as f32),
                body: lattice(MaterialKind::Glass, 3, 14, 12.0),
                pins: Pins {
                    center: true,
                    ..default()
                },
                points: 100,
                credit: Credit::Player(scorer),
                destroyed_at: 0.3,
                ..default()
            });
        }
    }
    for y in [-160.0, 0.0, 160.0] {
        elements.push(Element {
            name: "Obstacle".into(),
            pos: Vec2::new(0.0, y),
            points: 50,
            body: lattice(MaterialKind::Wood, 4, 4, 15.0),
            pins: Pins {
                center: true,
                ..default()
            },
            ..default()
        });
    }
    elements.push(Element {
        name: "Ball".into(),
        pos: Vec2::new(-120.0, 0.0),
        velocity: Vec2::new(-600.0, 240.0),
        body: Body::Lattice {
            material: MaterialKind::Steel,
            cols: 5,
            rows: 5,
            cell: 9.0,
            round: true,
        },
        keep_speed: 600.0,
        speed_ramp: 20.0,
        respawn: true,
        destroyed_at: 0.35,
        bounce: Some(1.0),
        friction: Some(0.0),
        ..default()
    });
    Level {
        name: "pong".into(),
        gravity: 0.0,
        substeps: 24,
        view: Vec2::new(1820.0, 1020.0),
        bounds: Vec2::new(950.0, 560.0),
        gun: false,
        elements,
        ..default()
    }
}

/// Where levels and highscores live: files in `levels/` natively, `localStorage` on the web.
mod storage {
    #[cfg(not(target_arch = "wasm32"))]
    mod imp {
        use std::path::PathBuf;

        const DIR: &str = "levels";

        fn path(key: &str) -> PathBuf {
            PathBuf::from(DIR).join(format!("{key}.ron"))
        }

        pub fn read(key: &str) -> Option<String> {
            std::fs::read_to_string(path(key)).ok()
        }

        pub fn write(key: &str, text: &str) -> Result<String, String> {
            std::fs::create_dir_all(DIR).map_err(|e| e.to_string())?;
            let path = path(key);
            std::fs::write(&path, text).map_err(|e| e.to_string())?;
            Ok(path.display().to_string())
        }

        pub fn keys() -> Vec<String> {
            let Ok(dir) = std::fs::read_dir(DIR) else {
                return vec![];
            };
            dir.filter_map(|entry| {
                let path = entry.ok()?.path();
                if path.extension()? != "ron" {
                    return None;
                }
                Some(path.file_stem()?.to_string_lossy().into_owned())
            })
            .collect()
        }
    }

    #[cfg(target_arch = "wasm32")]
    mod imp {
        const PREFIX: &str = "demolition-pong/";

        fn storage() -> Option<web_sys::Storage> {
            web_sys::window()?.local_storage().ok()?
        }

        pub fn read(key: &str) -> Option<String> {
            storage()?.get_item(&format!("{PREFIX}{key}")).ok()?
        }

        pub fn write(key: &str, text: &str) -> Result<String, String> {
            let storage = storage().ok_or("no localStorage")?;
            storage
                .set_item(&format!("{PREFIX}{key}"), text)
                .map_err(|e| format!("{e:?}"))?;
            Ok(format!("browser storage ({key})"))
        }

        pub fn keys() -> Vec<String> {
            let Some(storage) = storage() else {
                return vec![];
            };
            let len = storage.length().unwrap_or(0);
            (0..len)
                .filter_map(|i| storage.key(i).ok()?)
                .filter_map(|key| key.strip_prefix(PREFIX).map(str::to_owned))
                .collect()
        }
    }

    pub use imp::*;
}

const HIGHSCORE_KEY: &str = "highscores";
const LEVEL_PREFIX: &str = "level-";

fn pretty<T: Serialize>(value: &T) -> Result<String, String> {
    ron::ser::to_string_pretty(value, ron::ser::PrettyConfig::default()).map_err(|e| e.to_string())
}

/// The level as RON text, for saving or sharing.
pub fn to_ron(level: &Level) -> Result<String, String> {
    pretty(level)
}

pub fn from_ron(text: &str) -> Result<Level, String> {
    ron::from_str(text).map_err(|e| e.to_string())
}

/// Where share links point when not running in a browser.
const PUBLIC_URL: &str = "https://barafael.github.io/demolition/";

/// The page links should open: this page in the browser, the public site otherwise.
fn share_base() -> String {
    #[cfg(target_arch = "wasm32")]
    if let Some(location) = web_sys::window().map(|w| w.location())
        && let (Ok(origin), Ok(path)) = (location.origin(), location.pathname())
    {
        return format!("{origin}{path}");
    }
    PUBLIC_URL.to_string()
}

/// A link that opens the level, playing, in the web version. The level travels in the URL,
/// deflate-compressed and base64url-encoded, so nothing has to be hosted.
pub fn to_link(level: &Level) -> Result<String, String> {
    let text = ron::to_string(level).map_err(|e| e.to_string())?;
    let packed = miniz_oxide::deflate::compress_to_vec(text.as_bytes(), 9);
    Ok(format!(
        "{}?l={}&play",
        share_base(),
        base64url::encode(&packed)
    ))
}

/// Reads a level from a share link, or just its `l=` data.
pub fn from_link(link: &str) -> Result<Level, String> {
    let link = link.trim();
    let data = link
        .split(['?', '&', '#'])
        .find_map(|part| part.strip_prefix("l="))
        .unwrap_or(link);
    let packed = base64url::decode(data).ok_or("not a level link")?;
    let text = miniz_oxide::inflate::decompress_to_vec_with_limit(&packed, 8 << 20)
        .map_err(|e| format!("damaged level link ({e:?})"))?;
    from_ron(std::str::from_utf8(&text).map_err(|e| e.to_string())?)
}

/// Unpadded base64 with the URL-safe alphabet, so links need no escaping.
mod base64url {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

    pub fn encode(bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
        for chunk in bytes.chunks(3) {
            let n = chunk
                .iter()
                .enumerate()
                .fold(0u32, |n, (i, &b)| n | (b as u32) << (16 - 8 * i));
            for i in 0..=chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            }
        }
        out
    }

    pub fn decode(text: &str) -> Option<Vec<u8>> {
        let digits: Vec<u32> = text
            .bytes()
            .map(|c| ALPHABET.iter().position(|&a| a == c).map(|v| v as u32))
            .collect::<Option<_>>()?;
        let mut out = Vec::with_capacity(digits.len() * 3 / 4);
        for chunk in digits.chunks(4) {
            if chunk.len() < 2 {
                return None;
            }
            let n = chunk
                .iter()
                .enumerate()
                .fold(0u32, |n, (i, &d)| n | d << (18 - 6 * i));
            for i in 0..chunk.len() - 1 {
                out.push((n >> (16 - 8 * i)) as u8);
            }
        }
        Some(out)
    }
}

/// Saves the level and returns where it went.
pub fn save(level: &Level) -> Result<String, String> {
    storage::write(&format!("{LEVEL_PREFIX}{}", level.name), &to_ron(level)?)
}

pub fn load(name: &str) -> Result<Level, String> {
    let text = storage::read(&format!("{LEVEL_PREFIX}{name}")).ok_or("not found")?;
    from_ron(&text)
}

/// Names of the saved levels.
pub fn list() -> Vec<String> {
    let mut names: Vec<String> = storage::keys()
        .into_iter()
        .filter_map(|key| key.strip_prefix(LEVEL_PREFIX).map(str::to_owned))
        .collect();
    names.sort();
    names
}

/// Best score per level name, persisted next to the levels.
#[derive(Resource, Default, Serialize, Deserialize)]
pub struct Highscores(pub HashMap<String, i32>);

impl Highscores {
    pub fn load() -> Self {
        storage::read(HIGHSCORE_KEY)
            .and_then(|text| ron::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Ok(text) = pretty(self) {
            let _ = storage::write(HIGHSCORE_KEY, &text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_round_trip_through_ron() {
        for level in [preset_empty(), preset_lab(), preset_pong()] {
            let text =
                ron::ser::to_string_pretty(&level, ron::ser::PrettyConfig::default()).unwrap();
            let back: Level = ron::from_str(&text).unwrap();
            assert_eq!(back, level);
        }
    }

    #[test]
    fn levels_round_trip_through_share_links() {
        for level in [preset_empty(), preset_lab(), preset_pong()] {
            let link = to_link(&level).unwrap();
            assert!(
                link.len() < 8000,
                "{} link is {} characters",
                level.name,
                link.len()
            );
            assert_eq!(from_link(&link).unwrap(), level);
            // Just the data works too.
            let data = link.split("l=").nth(1).unwrap().split('&').next().unwrap();
            assert_eq!(from_link(data).unwrap(), level);
        }
    }

    #[test]
    fn base64url_handles_every_length() {
        for len in 0..20 {
            let bytes: Vec<u8> = (0..len).map(|i| (i * 37 + 11) as u8).collect();
            assert_eq!(
                base64url::decode(&base64url::encode(&bytes)).unwrap(),
                bytes
            );
        }
        assert!(base64url::decode("not+valid").is_none());
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let level: Level = ron::from_str("(name: \"tiny\", elements: [(name: \"box\")])").unwrap();
        assert_eq!(level.name, "tiny");
        assert_eq!(level.elements[0].pins.every, 1);
        assert_eq!(level.gravity, Level::default().gravity);
    }
}
