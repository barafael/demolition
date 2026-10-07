//! Grabbing: in play mode, right-drag picks up any free piece - balls, crates, whole
//! structures; left-drag too when the level has no gun to claim the button. What is held is
//! the piece still bonded to the body under the cursor, not every fragment its element ever
//! shed. While held, all of its bodies get the same velocity toward the cursor, which strains
//! no internal bonds but does strain pins: a hard yank tears a structure loose. On release the
//! piece keeps its velocity, so a fast drag becomes a throw.

use std::collections::{HashMap, HashSet};

use avian2d::prelude::*;
use bevy::prelude::*;
use bevy::window::{CursorIcon, PrimaryWindow, SystemCursorIcon};

use crate::lattice::{Bond, Doomed, Group};
use crate::level::Level;
use crate::level::Player;
use crate::play::{CursorWorld, Driven, ElementRoot, Grab, HitTag, Mode};
use crate::ui::{PointerOwner, world_pointer};
use crate::visuals::aim_and_fire;

/// How fast the grabbed element chases the cursor.
const SNAP: f32 = 20.0;
/// Fastest speed a grab can impart; also the strongest possible throw.
const GRAB_SPEED: f32 = 1400.0;
/// Ring color of the held piece.
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

/// The body under the cursor that the mouse may move, and its element: dynamic, and not
/// part of an element the player already drives (a paddle).
fn pick(
    cursor: Vec2,
    spatial: &SpatialQuery,
    groups: &Query<&Group>,
    candidates: &Query<(&Group, &RigidBody)>,
    driven: &Query<Entity, With<Driven>>,
) -> Option<(Entity, Entity)> {
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
        return Some((root, hit));
    }
    None
}

/// The bodies still bonded to `start` (itself included): the piece the mouse actually holds.
fn piece(
    start: Entity,
    root: Entity,
    bonds: &Query<(&FixedJoint, &Bond, &Group)>,
) -> HashSet<Entity> {
    let mut neighbours: HashMap<Entity, Vec<Entity>> = HashMap::new();
    for (joint, bond, group) in bonds {
        // Pins hold on to other things; only bonds within the element make up the piece.
        if group.0 == root && bond.material.is_some() {
            neighbours.entry(joint.body1).or_default().push(joint.body2);
            neighbours.entry(joint.body2).or_default().push(joint.body1);
        }
    }
    let mut seen = HashSet::from([start]);
    let mut todo = vec![start];
    while let Some(body) = todo.pop() {
        for &next in neighbours.get(&body).into_iter().flatten() {
            if seen.insert(next) {
                todo.push(next);
            }
        }
    }
    seen
}

/// Centroid (unweighted) and bounding radius of some bodies.
fn extent(
    bodies: &HashSet<Entity>,
    positions: &Query<&Position, Without<Doomed>>,
) -> Option<(Vec2, f32)> {
    let points: Vec<Vec2> = bodies
        .iter()
        .filter_map(|&b| positions.get(b).ok().map(|p| p.0))
        .collect();
    if points.is_empty() {
        return None;
    }
    let center = points.iter().sum::<Vec2>() / points.len() as f32;
    let radius = points
        .iter()
        .map(|p| p.distance(center))
        .fold(0.0, f32::max);
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
    bonds: Query<(&FixedJoint, &Bond, &Group)>,
    positions: Query<&Position, Without<Doomed>>,
    mut tags: Query<&mut HitTag>,
    mut grab: ResMut<Grab>,
) {
    if grab.body.is_some() {
        if !grab_held(&buttons, &level) {
            grab.release();
        }
        return;
    }
    let press = buttons.just_pressed(MouseButton::Right)
        || (buttons.just_pressed(MouseButton::Left) && !level.gun);
    if press
        && let Some(cursor) = cursor.0
        && let Some((root, body)) = pick(cursor, &spatial, &groups, &candidates, &driven)
        && let Some((center, _)) = extent(&piece(body, root, &bonds), &positions)
    {
        grab.root = Some(root);
        grab.body = Some(body);
        // What the player throws or swings counts as their hit (the mouse is player one's).
        if let Ok(mut tag) = tags.get_mut(root)
            && tag.owner.is_none()
        {
            tag.last_hit = Some(Player::One);
        }
        // Keep the pick point, so the piece doesn't snap its center onto the cursor.
        grab.offset = center - cursor;
    }
}

/// Steers the held piece: every body gets the same velocity toward the grab point, which
/// strains no internal bonds but lets the piece pull against its pins.
fn drag_grab(
    mut commands: Commands,
    buttons: Res<ButtonInput<MouseButton>>,
    cursor: Res<CursorWorld>,
    level: Res<Level>,
    roots: Query<&ElementRoot>,
    bonds: Query<(&FixedJoint, &Bond, &Group)>,
    positions: Query<&Position, Without<Doomed>>,
    mut velocities: Query<(&mut LinearVelocity, Has<Sleeping>), (With<RigidBody>, Without<Doomed>)>,
    mut grab: ResMut<Grab>,
) {
    let (Some(root), Some(body)) = (grab.root, grab.body) else {
        return;
    };
    if !grab_held(&buttons, &level)
        // The element was scored as destroyed, or the grabbed body is gone (restart, debris
        // expiry).
        || roots.get(root).is_ok_and(|root| root.destroyed)
        || !positions.contains(body)
    {
        grab.release();
        return;
    }
    let held = piece(body, root, &bonds);
    let (Some(cursor), Some((centroid, _))) = (cursor.0, extent(&held, &positions)) else {
        return;
    };
    // Keep the grab inside the level, so nothing is dragged out of reach of the cull.
    let lim = (level.bounds - Vec2::splat(40.0)).max(Vec2::splat(40.0));
    let target = (cursor + grab.offset).clamp(-lim, lim);
    let velocity = ((target - centroid) * SNAP).clamp_length_max(GRAB_SPEED);
    for &entity in &held {
        if let Ok((mut vel, asleep)) = velocities.get_mut(entity) {
            if asleep {
                commands.queue(WakeBody(entity));
            }
            vel.0 = velocity;
        }
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
    bonds: Query<(&FixedJoint, &Bond, &Group)>,
    positions: Query<&Position, Without<Doomed>>,
) {
    let held = grab.root.zip(grab.body);
    let target = held.or_else(|| {
        cursor
            .0
            .and_then(|cursor| pick(cursor, &spatial, &groups, &candidates, &driven))
    });
    if let Some((root, body)) = target
        && let Some((center, radius)) = extent(&piece(body, root, &bonds), &positions)
    {
        let color = if held.is_some() {
            GRAB_COLOR
        } else {
            Color::srgba(1.0, 1.0, 1.0, 0.45)
        };
        gizmos.circle_2d(center, radius + 8.0, color);
        if held.is_some()
            && let Some(cursor) = cursor.0
        {
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
    let icon = if grab.body.is_some() {
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
