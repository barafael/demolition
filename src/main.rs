mod editor;
mod effects;
mod fracture;
mod grab;
mod gun;
mod lattice;
mod level;
mod materials;
mod play;
mod probe;
mod ui;
mod visuals;

use avian2d::prelude::*;
use bevy::prelude::*;

use fracture::{Breaks, Stats, plasticity};
use gun::{Gun, tick_lifetimes};
use lattice::{WorldAnchor, despawn_doomed, sync_compliance};
use level::{Highscores, Level};
use materials::Materials;
use play::PlayPlugin;

/// Everything that makes the simulation run, without rendering or input, so the probe can
/// drive it headlessly.
pub struct SimPlugin;

impl Plugin for SimPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((PhysicsPlugins::default().with_length_unit(10.0), PlayPlugin))
            .init_resource::<Level>()
            .init_resource::<Materials>()
            .init_resource::<Gun>()
            .init_resource::<Stats>()
            .init_resource::<Breaks>()
            .add_systems(Update, (tick_lifetimes, sync_compliance))
            .add_systems(Last, despawn_doomed)
            .add_systems(
                FixedPostUpdate,
                plasticity
                    .after(PhysicsSystems::StepSimulation)
                    .before(PhysicsSystems::Writeback),
            );
        // Created up front: the initial state transition can run before `Startup`.
        let anchor = app
            .world_mut()
            .spawn((RigidBody::Static, Transform::default()))
            .id();
        app.insert_resource(WorldAnchor(anchor));
    }
}

/// Which level to open and whether to start playing right away.
/// Native: `--level <preset or saved name> --play`. Web: `?level=<name>&play`.
struct Launch {
    level: Option<String>,
    /// A share link or its data (`l=` in the URL, `--link` natively).
    link: Option<String>,
    play: bool,
}

impl Launch {
    #[cfg(not(target_arch = "wasm32"))]
    fn read() -> Self {
        let args: Vec<String> = std::env::args().collect();
        let value = |flag: &str| {
            args.iter()
                .position(|a| a == flag)
                .and_then(|i| args.get(i + 1).cloned())
        };
        Self {
            level: value("--level"),
            link: value("--link"),
            play: args.iter().any(|a| a == "--play"),
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn read() -> Self {
        let search = web_sys::window()
            .and_then(|w| w.location().search().ok())
            .unwrap_or_default();
        let params: Vec<(&str, &str)> = search
            .trim_start_matches('?')
            .split('&')
            .filter(|p| !p.is_empty())
            .map(|p| p.split_once('=').unwrap_or((p, "")))
            .collect();
        Self {
            level: params
                .iter()
                .find(|(k, _)| *k == "level")
                .map(|(_, v)| v.to_string()),
            link: params
                .iter()
                .find(|(k, _)| *k == "l")
                .map(|(_, v)| v.to_string()),
            play: params.iter().any(|(k, _)| *k == "play"),
        }
    }

    fn level(&self) -> Level {
        if let Some(link) = &self.link {
            match level::from_link(link) {
                Ok(level) => return level,
                Err(e) => warn!("could not open the shared level: {e}"),
            }
        }
        match self.level.as_deref() {
            None | Some("lab") => level::preset_lab(),
            Some("pong") => level::preset_pong(),
            Some("tower") => level::preset_tower(),
            Some("wreck") => level::preset_wreck(),
            Some("domino") => level::preset_domino(),
            Some("pyramid") => level::preset_pyramid(),
            Some("empty") => level::preset_empty(),
            Some(name) => level::load(name).unwrap_or_else(|e| {
                warn!("could not load level {name}: {e}");
                level::preset_lab()
            }),
        }
    }
}

fn main() {
    let arg = |flag: &str| std::env::args().any(|a| a == flag);
    if arg("--bench") {
        return probe::bench();
    }
    if arg("--diag") {
        return probe::diag();
    }
    if arg("--probe") {
        return probe::run();
    }
    if arg("--tnt") {
        return probe::tnt();
    }
    if arg("--rest") {
        return probe::rest_speeds();
    }
    if arg("--pong") {
        return probe::pong();
    }

    let launch = Launch::read();
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "Demolition Lab".into(),
            resolution: (1600, 900).into(),
            // On the web: render into the page's canvas and follow its size.
            canvas: Some("#bevy".into()),
            fit_canvas_to_parent: true,
            // Keep Tab, Space and arrow keys from scrolling or moving focus in the page.
            prevent_default_event_handling: true,
            ..default()
        }),
        ..default()
    }))
    .add_plugins((
        SimPlugin,
        visuals::VisualsPlugin,
        grab::GrabPlugin,
        effects::EffectsPlugin,
        editor::EditorPlugin,
        ui::UiPlugin,
    ))
    .insert_resource(launch.level())
    .insert_resource(Highscores::load());
    if launch.play {
        app.insert_state(play::Mode::Play);
    }
    app.run();
}
