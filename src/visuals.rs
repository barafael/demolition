//! Rendering, gizmo overlays and play-mode mouse/keyboard input for the windowed app.

use avian2d::prelude::*;
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::prelude::*;
use bevy_egui::input::{egui_wants_any_keyboard_input, egui_wants_any_pointer_input};

use crate::fracture::Breaks;
use crate::gun::{Ammo, Gun, MUZZLE, Round, fire};
use crate::lattice::{Bond, Cell, WorldAnchor};
use crate::level::Level;
use crate::materials::Materials;
use crate::play::{ClearDebris, CursorWorld, Mode, Restart};

pub struct VisualsPlugin;

#[derive(Resource)]
pub struct View {
    pub stress_overlay: bool,
    pub trajectory: bool,
}

impl Default for View {
    fn default() -> Self {
        Self {
            stress_overlay: false,
            trajectory: true,
        }
    }
}

impl Plugin for VisualsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ClearColor(Color::srgb(0.07, 0.07, 0.09)))
            .init_resource::<View>()
            .add_systems(Startup, |mut commands: Commands| {
                commands.spawn(Camera2d);
            })
            .add_systems(PreUpdate, update_cursor)
            .add_systems(
                Update,
                (
                    toggle_mode.run_if(not(egui_wants_any_keyboard_input)),
                    (
                        aim_and_fire.run_if(not(egui_wants_any_pointer_input)),
                        play_hotkeys.run_if(not(egui_wants_any_keyboard_input)),
                        draw_gun,
                    )
                        .run_if(in_state(Mode::Play)),
                    attach_round_meshes,
                    tint_cells,
                    draw_bonds,
                    draw_breaks,
                ),
            );
    }
}

fn update_cursor(
    windows: Query<&Window>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    mut cursor: ResMut<CursorWorld>,
) {
    cursor.0 = (|| {
        let window = windows.single().ok()?;
        let (camera, transform) = cameras.single().ok()?;
        camera
            .viewport_to_world_2d(transform, window.cursor_position()?)
            .ok()
    })();
}

fn toggle_mode(
    keys: Res<ButtonInput<KeyCode>>,
    mode: Res<State<Mode>>,
    mut next: ResMut<NextState<Mode>>,
) {
    if keys.just_pressed(KeyCode::Tab) {
        next.set(match mode.get() {
            Mode::Edit => Mode::Play,
            Mode::Play => Mode::Edit,
        });
    }
}

fn aim_and_fire(
    mut commands: Commands,
    buttons: Res<ButtonInput<MouseButton>>,
    scroll: Res<AccumulatedMouseScroll>,
    cursor: Res<CursorWorld>,
    time: Res<Time>,
    level: Res<Level>,
    materials: Res<Materials>,
    anchor: Res<WorldAnchor>,
    mut gun: ResMut<Gun>,
) {
    if !level.gun {
        return;
    }
    if scroll.delta.y != 0.0 {
        gun.speed = (gun.speed * 1.1f32.powf(scroll.delta.y.signum())).clamp(50.0, 6000.0);
    }
    let Some(cursor) = cursor.0 else {
        return;
    };
    if buttons.pressed(MouseButton::Right) {
        gun.pos = cursor;
        return;
    }
    if let Some(dir) = (cursor - gun.pos).try_normalize() {
        gun.dir = dir;
    }

    gun.cooldown -= time.delta_secs();
    let shoot = if gun.auto_rate > 0.0 {
        buttons.pressed(MouseButton::Left) && gun.cooldown <= 0.0
    } else {
        buttons.just_pressed(MouseButton::Left)
    };
    if shoot {
        fire(&mut commands, &materials, anchor.0, &gun);
        gun.cooldown = 1.0 / gun.auto_rate.max(0.001);
    }
}

fn play_hotkeys(
    keys: Res<ButtonInput<KeyCode>>,
    mut gun: ResMut<Gun>,
    mut view: ResMut<View>,
    mut time: ResMut<Time<Virtual>>,
    mut clear: ResMut<ClearDebris>,
    mut restart: ResMut<Restart>,
) {
    let digits = [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
    ];
    for (key, ammo) in digits.into_iter().zip(Ammo::ALL) {
        if keys.just_pressed(key) {
            gun.ammo = ammo;
            gun.speed = ammo.default_speed();
        }
    }
    if keys.just_pressed(KeyCode::KeyR) {
        restart.0 = true;
    }
    if keys.just_pressed(KeyCode::KeyC) {
        clear.0 = true;
    }
    if keys.just_pressed(KeyCode::KeyB) {
        view.stress_overlay = !view.stress_overlay;
    }
    if keys.just_pressed(KeyCode::KeyT) {
        view.trajectory = !view.trajectory;
    }
    if keys.just_pressed(KeyCode::Space) {
        if time.is_paused() {
            time.unpause();
        } else {
            time.pause();
        }
    }
    if keys.just_pressed(KeyCode::KeyM) {
        let speed = if time.relative_speed() < 1.0 {
            1.0
        } else {
            0.2
        };
        time.set_relative_speed(speed);
    }
}

fn attach_round_meshes(
    mut commands: Commands,
    added: Query<(Entity, &Round), Added<Round>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut color_materials: ResMut<Assets<ColorMaterial>>,
) {
    for (entity, round) in &added {
        commands.entity(entity).insert((
            Mesh2d(meshes.add(Circle::new(round.radius))),
            MeshMaterial2d(color_materials.add(round.color)),
        ));
    }
}

fn tint_cells(materials: Res<Materials>, mut cells: Query<(&Cell, &mut Sprite)>) {
    let cracked = Color::srgb(0.25, 0.05, 0.05);
    for (cell, mut sprite) in &mut cells {
        let base = materials.get(cell.material).color;
        sprite.color = base.mix(&cracked, cell.damage.clamp(0.0, 1.0) * 0.85);
    }
}

fn draw_gun(
    mut gizmos: Gizmos,
    gun: Res<Gun>,
    level: Res<Level>,
    gravity: Res<Gravity>,
    view: Res<View>,
) {
    if !level.gun {
        return;
    }
    let muzzle = gun.pos + gun.dir * MUZZLE;
    gizmos.circle_2d(gun.pos, 14.0, Color::srgb(0.8, 0.8, 0.85));
    gizmos.line_2d(
        gun.pos + gun.dir * 14.0,
        muzzle,
        Color::srgb(0.8, 0.8, 0.85),
    );

    if view.trajectory {
        let v = gun.dir * gun.speed;
        let dots = 40;
        // Show ~0.8 s of flight, or 1600 px for fast shots, whichever comes first.
        let duration = (1600.0 / gun.speed).min(0.8);
        for k in 1..=dots {
            let t = duration * k as f32 / dots as f32;
            let p = muzzle + v * t + gravity.0 * (0.5 * t * t);
            let fade = 1.0 - k as f32 / dots as f32;
            gizmos.circle_2d(p, 1.5, Color::srgba(1.0, 0.9, 0.4, 0.7 * fade));
        }
    }
}

fn strain_color(x: f32) -> Color {
    let x = x.clamp(0.0, 1.0);
    if x < 0.5 {
        Color::srgb(0.2 + 1.6 * x, 0.9, 0.3)
    } else {
        Color::srgb(1.0, 0.9 - 1.6 * (x - 0.5), 0.2)
    }
}

fn draw_bonds(
    mut gizmos: Gizmos,
    view: Res<View>,
    materials: Res<Materials>,
    joints: Query<(&FixedJoint, &Bond)>,
    bodies: Query<(&Position, &Rotation)>,
) {
    for (joint, bond) in &joints {
        let pin = bond.material.is_none();
        if !pin && !view.stress_overlay {
            continue;
        }
        let Ok([(p1, r1), (p2, _)]) = bodies.get_many([joint.body1, joint.body2]) else {
            continue;
        };
        if pin {
            // Draw the bolt where it is fixed to its anchor, plus a line to the body if bent.
            let JointAnchor::Local(local) = joint.frame1.anchor else {
                continue;
            };
            let at = p1.0 + *r1 * local;
            let s = bond.strength(&materials);
            let wear = (bond.damage / s.ductility.max(1e-3)).max(bond.strain);
            let color = strain_color(wear);
            gizmos.circle_2d(at, 3.0, color);
            if view.stress_overlay {
                gizmos.line_2d(at, p2.0, color);
            }
        } else {
            gizmos.line_2d(p1.0, p2.0, strain_color(bond.strain));
        }
    }
}

fn draw_breaks(mut gizmos: Gizmos, mut breaks: ResMut<Breaks>, time: Res<Time<Virtual>>) {
    const FLASH: f32 = 0.35;
    let now = time.elapsed_secs();
    breaks.0.retain(|(_, t)| now - *t < FLASH);
    for (pos, t) in &breaks.0 {
        let age = (now - t) / FLASH;
        gizmos.circle_2d(
            *pos,
            2.0 + 10.0 * age,
            Color::srgba(1.0, 0.85, 0.3, 1.0 - age),
        );
    }
}
