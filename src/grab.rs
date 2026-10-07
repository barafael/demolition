//! Grabbing: in play mode, right-drag picks up any free element - balls, crates, whole
//! structures; left-drag too when the level has no gun to claim the button. While held, every
//! body of the element is given the same velocity toward the cursor, which strains no internal
//! bonds but does strain pins: a hard yank tears a structure loose. On release the element
//! keeps its velocity, so a fast drag becomes a throw.

use avian2d::prelude::*;
use bevy::prelude::*;
use bevy::window::{CursorIcon, PrimaryWindow, SystemCursorIcon};

use crate::lattice::{Doomed, Group};
use crate::level::Level;
use crate::play::{CursorWorld, Driven, ElementRoot, Grab, Mode};
use crate::ui::{PointerOwner, world_pointer};
use crate::visuals::aim_and_fire;

/// How fast the grabbed element chases the cursor.
const SNAP: f32 = 20.0;
/// Fastest speed a grab can impart; also the strongest possible throw.
const GRAB_SPEED: f32 = 1400.0;
/// Gold, like a staged move in a turn-based game: this one is held.
const GRAB_COLOR: Color = Color::srgb(1.0, 0.82, 0.30);

pub struct GrabPlugin;

impl Plugin for GrabPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                (grab_pointer, drag_grab, show_grab)
                    .chain()
                    // Claiming the press has to win over moving the gun with the same button.
                    .before(aim_and_fire)
                    .run_if(world_pointer),
                grab_cursor,
            )
                .run_if(in_state(Mode::Play)),
        )
        // The cursor systems stop running outside Play mode, so the hand icon could stick.
        .add_systems(OnExit(Mode::Play), reset_cursor);
    }
}

/// The window entity starts without the component, so this stays quiet until it has one.
fn reset_cursor(mut icons: Query<&mut CursorIcon>) {
    if let Ok(mut icon) = icons.single_mut() {
        *icon = CursorIcon::System(SystemCursorIcon::Default);
    }
}

/// Whether a grab gesture is still in progress. The right button always grabs; the left one
/// only when the level has no gun, where firing wouldn't claim it.
fn grab_held(buttons: &ButtonInput<MouseButton>, level: &Level) -> bool {
    buttons.pressed(MouseButton::Right) || (!level.gun && buttons.pressed(MouseButton::Left))
}

/// The element root under the cursor whose bodies the mouse may move: dynamic, and not part
/// of an element the player already drives (a paddle).
fn pick(
    cursor: Vec2,
    spatial: &SpatialQuery,
    groups: &Query<&Group>,
    candidates: &Query<(&Group, &RigidBody)>,
    driven: &Query<Entity, With<Driven>>,
) -> Option<Entity> {
    for hit in spatial.point_intersections(cursor, &SpatialQueryFilter::DEFAULT) {
        let Ok((group, body)) = candidates.get(hit) else {
            continue;
        };
        if !body.is_dynamic() {
            continue;
        }
        let root = group.0;
        // Paddle cells are dynamic, but their element hangs from a driven carrier.
        if driven
            .iter()
            .any(|carrier| groups.get(carrier).is_ok_and(|g| g.0 == root))
        {
            continue;
        }
        return Some(root);
    }
    None
}

/// Centroid (unweighted) and bounding radius of an element's bodies.
fn extent(
    root: Entity,
    members: &Query<(&Group, &Position), Without<Doomed>>,
) -> Option<(Vec2, f32)> {
    let mut sum = Vec2::ZERO;
    let mut n = 0;
    let mut radius: f32 = 0.0;
    for (_, pos) in members.iter().filter(|(group, _)| group.0 == root) {
        sum += pos.0;
        n += 1;
    }
    if n == 0 {
        return None;
    }
    let center = sum / n as f32;
    for (_, pos) in members.iter().filter(|(group, _)| group.0 == root) {
        radius = radius.max(pos.0.distance(center));
    }
    Some((center, radius))
}

fn grab_pointer(
    buttons: Res<ButtonInput<MouseButton>>,
    cursor: Res<CursorWorld>,
    level: Res<Level>,
    spatial: SpatialQuery,
    groups: Query<&Group>,
    candidates: Query<(&Group, &RigidBody)>,
    driven: Query<Entity, With<Driven>>,
    members: Query<(&Group, &Position), Without<Doomed>>,
    mut grab: ResMut<Grab>,
) {
    if grab.root.is_some() {
        if !grab_held(&buttons, &level) {
            grab.root = None;
        }
        return;
    }
    let press = buttons.just_pressed(MouseButton::Right)
        || (buttons.just_pressed(MouseButton::Left) && !level.gun);
    if press
        && let Some(cursor) = cursor.0
        && let Some(root) = pick(cursor, &spatial, &groups, &candidates, &driven)
        && let Some((center, _)) = extent(root, &members)
    {
        grab.root = Some(root);
        // Keep the pick point, so the element doesn't snap its center onto the cursor.
        grab.offset = center - cursor;
    }
}

/// Steers the held element: every body gets the same velocity toward the grab point, which
/// strains no internal bonds but lets the element pull against its pins.
fn drag_grab(
    mut commands: Commands,
    buttons: Res<ButtonInput<MouseButton>>,
    cursor: Res<CursorWorld>,
    level: Res<Level>,
    roots: Query<&ElementRoot>,
    mut members: Query<
        (
            Entity,
            &Group,
            &Position,
            &mut LinearVelocity,
            Has<Sleeping>,
        ),
        (With<RigidBody>, Without<Doomed>),
    >,
    mut grab: ResMut<Grab>,
) {
    let Some(root) = grab.root else { return };
    if !grab_held(&buttons, &level)
        // The element fell apart (scored) or despawned (restart, debris expiry).
        || roots.get(root).is_ok_and(|root| root.destroyed)
    {
        grab.root = None;
        return;
    }
    let held: Vec<_> = members
        .iter_mut()
        .filter(|(_, group, ..)| group.0 == root)
        .collect();
    if held.is_empty() {
        grab.root = None;
        return;
    }
    let Some(cursor) = cursor.0 else { return };
    let centroid = held.iter().map(|(_, _, pos, _, _)| pos.0).sum::<Vec2>() / held.len() as f32;
    // Keep the grab inside the level, so nothing is dragged out of reach of the cull.
    let lim = (level.bounds - Vec2::splat(40.0)).max(Vec2::splat(40.0));
    let target = (cursor + grab.offset).clamp(-lim, lim);
    let velocity = ((target - centroid) * SNAP).clamp_length_max(GRAB_SPEED);
    for (entity, _, _, mut vel, asleep) in held {
        if asleep {
            commands.queue(WakeBody(entity));
        }
        vel.0 = velocity;
    }
}

/// White ring around whatever a grab would pick up; gold ring and a grip line on the held one.
fn show_grab(
    mut gizmos: Gizmos,
    cursor: Res<CursorWorld>,
    grab: Res<Grab>,
    spatial: SpatialQuery,
    groups: Query<&Group>,
    candidates: Query<(&Group, &RigidBody)>,
    driven: Query<Entity, With<Driven>>,
    members: Query<(&Group, &Position), Without<Doomed>>,
) {
    let hover = cursor
        .0
        .and_then(|cursor| pick(cursor, &spatial, &groups, &candidates, &driven));
    if let Some(root) = grab.root.or(hover)
        && let Some((center, radius)) = extent(root, &members)
    {
        let held = grab.root == Some(root);
        let color = if held {
            GRAB_COLOR
        } else {
            Color::srgba(1.0, 1.0, 1.0, 0.45)
        };
        gizmos.circle_2d(center, radius + 8.0, color);
        if held && let Some(cursor) = cursor.0 {
            gizmos.line_2d(cursor, center, GRAB_COLOR.with_alpha(0.35));
        }
    }
}

/// The mouse cursor reflects what a click would do, even over the panels, where the ring
/// systems stay quiet.
fn grab_cursor(
    cursor: Res<CursorWorld>,
    grab: Res<Grab>,
    owner: Res<PointerOwner>,
    spatial: SpatialQuery,
    groups: Query<&Group>,
    candidates: Query<(&Group, &RigidBody)>,
    driven: Query<Entity, With<Driven>>,
    windows: Query<Entity, With<PrimaryWindow>>,
    mut icons: Query<&mut CursorIcon>,
    mut commands: Commands,
) {
    // The window entity starts without the component; inserting it makes winit switch icons.
    let Ok(window) = windows.single() else { return };
    let hover = !owner.over_ui()
        && cursor
            .0
            .is_some_and(|cursor| pick(cursor, &spatial, &groups, &candidates, &driven).is_some());
    let icon = if grab.root.is_some() {
        CursorIcon::System(SystemCursorIcon::Grabbing)
    } else if hover {
        CursorIcon::System(SystemCursorIcon::Grab)
    } else {
        CursorIcon::System(SystemCursorIcon::Default)
    };
    if let Ok(mut current) = icons.single_mut() {
        if *current != icon {
            *current = icon;
        }
    } else {
        commands.entity(window).insert(icon);
    }
}
