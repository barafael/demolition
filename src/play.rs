//! Play mode: spawns the edited `Level` into the physics world and runs element behaviours
//! (player control, scoring, keep-speed, respawning). Leaving Play mode removes it all again.

use std::collections::HashMap;

use avian2d::prelude::*;
use bevy::prelude::*;

use crate::fracture::{Breaks, Stats};
use crate::gun::{Gun, Projectile, Round};
use crate::lattice::{
    Bond, Cell, Detonating, Doomed, Group, GroupRoot, LatticeSpec, Pin, WorldAnchor, spawn_lattice,
    spawn_pin,
};
use crate::level::{Axis, Body, Control, Credit, Element, Highscores, Level, Player};
use crate::materials::Materials;

#[derive(States, Default, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Mode {
    #[default]
    Edit,
    Play,
}

/// Everything spawned while playing; removed when returning to the editor.
#[derive(Component)]
pub struct LevelEntity;

#[derive(Component)]
pub struct ElementRoot {
    /// Index into `Level::elements`.
    pub index: usize,
    /// Internal bonds at spawn time.
    pub bonds: usize,
    /// Pins at spawn time.
    pub pins: usize,
    pub destroyed: bool,
    /// Seconds since spawning.
    pub age: f32,
}

/// Gives the bodies of this element `CollidingEntities`, for keep-speed's contact check.
#[derive(Component)]
struct TrackContacts;

/// Makes the bodies of this element report collisions, so it can pass on its hit tag.
#[derive(Component)]
pub struct ReportHits;

/// Who an element or projectile belongs to, and which player last hit it.
#[derive(Component, Default)]
pub struct HitTag {
    pub owner: Option<Player>,
    pub last_hit: Option<Player>,
}

impl HitTag {
    /// The player this passes on to whatever it hits.
    fn carried(&self) -> Option<Player> {
        self.owner.or(self.last_hit)
    }
}

/// Adds per-body components requested by markers on group roots (spawned before their bodies
/// exist as queryable entities).
fn equip_members(
    mut commands: Commands,
    contacts: Query<Entity, Added<TrackContacts>>,
    reports: Query<Entity, Added<ReportHits>>,
    members: Query<(Entity, &Group), With<RigidBody>>,
) {
    if contacts.is_empty() && reports.is_empty() {
        return;
    }
    for (entity, group) in &members {
        if contacts.contains(group.0) {
            commands.entity(entity).insert(CollidingEntities::default());
        }
        if reports.contains(group.0) {
            commands.entity(entity).insert(CollisionEventsEnabled);
        }
    }
}

/// Passes hit tags along on contact: a paddle tags the ball, the ball tags what it hits.
fn propagate_hits(
    mut collisions: MessageReader<CollisionStart>,
    groups: Query<&Group>,
    mut tags: Query<&mut HitTag>,
) {
    for event in collisions.read() {
        let root = |e: Entity| groups.get(e).map_or(e, |g| g.0);
        let (a, b) = (root(event.collider1), root(event.collider2));
        if a == b {
            continue;
        }
        let Ok([mut ta, mut tb]) = tags.get_many_mut([a, b]) else {
            continue;
        };
        let (carried_a, carried_b) = (ta.carried(), tb.carried());
        if let Some(p) = carried_a
            && tb.owner.is_none()
        {
            tb.last_hit = Some(p);
        }
        if let Some(p) = carried_b
            && ta.owner.is_none()
        {
            ta.last_hit = Some(p);
        }
    }
}

/// A kinematic body moved by player input.
#[derive(Component, Clone, Copy)]
pub struct Driven {
    pub control: Control,
    pub axis: Axis,
    pub speed: f32,
    pub range: f32,
    /// Spawn position; the locked axis stays at this coordinate.
    pub home: Vec2,
}

#[derive(Resource, Default)]
pub struct Score {
    pub points: [i32; 2],
}

/// World-space cursor position, written by the windowed front-end.
#[derive(Resource, Default)]
pub struct CursorWorld(pub Option<Vec2>);

/// Set to rebuild the level from scratch while staying in Play mode.
#[derive(Resource, Default)]
pub struct Restart(pub bool);

#[derive(Resource, Default)]
struct Respawns(Vec<(usize, f32)>);

const RESPAWN_DELAY: f32 = 1.5;
/// Loose debris is removed after this long, so long sessions don't slow down as it piles up.
pub const DEBRIS_LIFETIME: f32 = 20.0;
/// Reach of an explosion of power 1, in pixels; it grows with the square root of the power.
const BLAST_RADIUS: f32 = 90.0;
/// Impulse an explosion of power 1 gives a body right next to it.
const BLAST_IMPULSE: f32 = 1500.0;
/// Sanity limit for stacked blasts from a whole charge going off at once.
const MAX_BLAST_SPEED: f32 = 12000.0;
/// Bodies a blast makes faster than this get swept CCD for a moment, so they can't tunnel
/// through walls.
const BLAST_CCD_SPEED: f32 = 1500.0;
const BLAST_CCD_SECONDS: f32 = 1.0;

/// Swept CCD that a blast added, and when to take it away again.
#[derive(Component)]
struct BlastCcd(f32);

/// Explosions this frame, for effects (flash, shake, particles).
#[derive(Resource, Default)]
pub struct Explosions(pub Vec<Explosion>);

#[derive(Clone, Copy)]
pub struct Explosion {
    pub pos: Vec2,
    pub radius: f32,
    pub power: f32,
}

pub struct PlayPlugin;

impl Plugin for PlayPlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<Mode>()
            .init_resource::<Score>()
            .init_resource::<CursorWorld>()
            .init_resource::<Restart>()
            .init_resource::<Respawns>()
            .init_resource::<ClearDebris>()
            .init_resource::<Explosions>()
            .init_resource::<Highscores>()
            .add_systems(PreUpdate, sync_level_materials)
            .add_systems(OnEnter(Mode::Play), enter_play)
            .add_systems(OnExit(Mode::Play), (clear_level, save_highscores))
            .add_systems(
                Update,
                (
                    apply_world_settings,
                    restart,
                    clear_debris,
                    equip_members,
                    propagate_hits,
                    drive,
                    keep_speed,
                    track_elements,
                    respawn,
                    detonate,
                    expire_blast_ccd,
                    age_debris,
                    cull_out_of_bounds,
                )
                    .chain()
                    .run_if(in_state(Mode::Play)),
            );
    }
}

/// The level owns its material table; systems read the `Materials` resource.
fn sync_level_materials(level: Res<Level>, mut materials: ResMut<Materials>) {
    if level.is_changed() && level.materials != *materials {
        *materials = level.materials.clone();
    }
}

/// Gravity and substeps follow the level while playing, so World settings apply live. Only
/// written when they differ: changing gravity wakes every sleeping body.
fn apply_world_settings(
    level: Res<Level>,
    mut gravity: ResMut<Gravity>,
    mut substeps: ResMut<SubstepCount>,
) {
    if !level.is_changed() {
        return;
    }
    let g = Vec2::NEG_Y * level.gravity;
    if gravity.0 != g {
        gravity.0 = g;
    }
    let n = level.substeps.max(1);
    if substeps.0 != n {
        substeps.0 = n;
    }
}

fn enter_play(
    mut commands: Commands,
    level: Res<Level>,
    materials: Res<Materials>,
    anchor: Res<WorldAnchor>,
    mut gravity: ResMut<Gravity>,
    mut substeps: ResMut<SubstepCount>,
    mut gun: ResMut<Gun>,
    mut score: ResMut<Score>,
    mut stats: ResMut<Stats>,
    mut breaks: ResMut<Breaks>,
    mut respawns: ResMut<Respawns>,
) {
    gravity.0 = Vec2::NEG_Y * level.gravity;
    substeps.0 = level.substeps.max(1);
    gun.pos = level.gun_pos;
    *score = Score::default();
    *stats = Stats::default();
    breaks.0.clear();
    respawns.0.clear();
    for (index, element) in level.elements.iter().enumerate() {
        spawn_element(&mut commands, &materials, anchor.0, element, index, level.resolution);
    }
}

fn clear_level(
    mut commands: Commands,
    spawned: Query<Entity, Or<(With<LevelEntity>, With<Group>, With<GroupRoot>)>>,
) {
    for entity in &spawned {
        commands.entity(entity).despawn();
    }
}

fn save_highscores(highscores: Res<Highscores>) {
    highscores.save();
}

fn restart(
    mut commands: Commands,
    mut request: ResMut<Restart>,
    spawned: Query<Entity, Or<(With<LevelEntity>, With<Group>, With<GroupRoot>)>>,
    level: Res<Level>,
    materials: Res<Materials>,
    anchor: Res<WorldAnchor>,
    mut score: ResMut<Score>,
    mut respawns: ResMut<Respawns>,
) {
    if !std::mem::take(&mut request.0) {
        return;
    }
    clear_level(commands.reborrow(), spawned);
    *score = Score::default();
    respawns.0.clear();
    for (index, element) in level.elements.iter().enumerate() {
        spawn_element(&mut commands, &materials, anchor.0, element, index, level.resolution);
    }
}

pub fn spawn_element(
    commands: &mut Commands,
    materials: &Materials,
    world_anchor: Entity,
    element: &Element,
    index: usize,
    resolution: f32,
) -> Entity {
    let root = commands
        .spawn((
            GroupRoot,
            ElementRoot {
                index,
                bonds: 0,
                pins: 0,
                destroyed: false,
                age: 0.0,
            },
            LevelEntity,
            HitTag {
                owner: element.owner,
                last_hit: None,
            },
            Name::new(element.name.clone()),
        ))
        .id();
    if element.owner.is_some() || element.keep_speed > 0.0 {
        commands.entity(root).insert(ReportHits);
    }
    let group = Group(root);
    let pose = element.pose();
    let transform = Transform::from_translation(element.pos.extend(0.0))
        .with_rotation(Quat::from_rotation_z(element.angle));
    let driven = (element.control != Control::None).then_some(Driven {
        control: element.control,
        axis: element.axis,
        speed: element.speed,
        range: element.range,
        home: element.pos,
    });

    // Pins attach to the static world, or to a kinematic carrier the player drives.
    let anchor = match (&element.body, driven) {
        (Body::Wall { .. }, _) | (_, None) => (world_anchor, Isometry2d::IDENTITY),
        (_, Some(driven)) => {
            let carrier = commands
                .spawn((RigidBody::Kinematic, transform, driven, group))
                .id();
            (carrier, pose)
        }
    };

    match &element.body {
        Body::Wall {
            width,
            height,
            color,
        } => {
            let mut wall = commands.spawn((
                if driven.is_some() {
                    RigidBody::Kinematic
                } else {
                    RigidBody::Static
                },
                Collider::rectangle(*width, *height),
                Friction::new(0.6),
                Sprite::from_color(Color::srgb_from_array(*color), Vec2::new(*width, *height)),
                transform.with_translation(element.pos.extend(-1.0)),
                group,
            ));
            if let Some(driven) = driven {
                wall.insert(driven);
            }
        }
        Body::Ball {
            radius,
            density,
            restitution,
            friction,
            color,
        } => {
            let restitution = match element.bounce {
                Some(b) => Restitution::new(b).with_combine_rule(CoefficientCombine::Max),
                None => Restitution::new(*restitution),
            };
            let ball = commands
                .spawn((
                    RigidBody::Dynamic,
                    Collider::circle(*radius),
                    ColliderDensity(*density),
                    restitution,
                    match element.friction {
                        Some(f) => Friction::new(f).with_combine_rule(CoefficientCombine::Min),
                        None => Friction::new(*friction),
                    },
                    LinearVelocity(element.velocity),
                    SweptCcd::default(),
                    CollidingEntities::default(),
                    CollisionEventsEnabled,
                    transform.with_translation(element.pos.extend(1.0)),
                    Round {
                        radius: *radius,
                        color: Color::srgb_from_array(*color),
                    },
                    group,
                ))
                .id();
            if driven.is_some() || element.keep_speed > 0.0 {
                commands.entity(ball).insert(SleepingDisabled);
            }
            if element.pins.any() || driven.is_some() {
                commands.entity(root).insert(ElementRoot {
                    index,
                    bonds: 0,
                    pins: 1,
                    destroyed: false,
                    age: 0.0,
                });
                spawn_pin(
                    commands,
                    materials,
                    anchor.0,
                    anchor.1,
                    ball,
                    pose,
                    Vec2::ZERO,
                    radius * 2.0,
                    group,
                );
            }
        }
        Body::Lattice { .. } => {
            let spec = lattice_spec(element, resolution).expect("lattice body");
            let (bonds, pins) = spawn_lattice(commands, materials, anchor.0, anchor.1, &spec, root);
            if element.keep_speed > 0.0 {
                commands.entity(root).insert(TrackContacts);
            }
            commands.entity(root).insert(ElementRoot {
                index,
                bonds,
                pins,
                destroyed: false,
                age: 0.0,
            });
        }
    }
    root
}

/// The lattice layout of a lattice element at the level's physics resolution, pins included.
pub fn lattice_spec(element: &Element, resolution: f32) -> Option<LatticeSpec> {
    let strain_length = match element.body {
        Body::Lattice { cell, .. } => cell.max(1.0),
        _ => return None,
    };
    let element = &element.at_resolution(resolution);
    let Body::Lattice {
        material,
        cols,
        rows,
        cell,
        round,
    } = element.body
    else {
        return None;
    };
    let mut spec = LatticeSpec {
        center: element.pos,
        angle: element.angle,
        cols: cols.max(1),
        rows: rows.max(1),
        cell: cell.max(0.1),
        strain_length,
        material,
        velocity: element.velocity,
        round,
        pins: vec![],
        ccd: element.velocity != Vec2::ZERO || element.keep_speed > 0.0,
        can_sleep: element.control == Control::None && element.keep_speed == 0.0,
        bounce: element.bounce,
        friction: element.friction,
    };
    let pins = element.lattice_pins(|i, j| spec.has_cell(i, j));
    spec.pins = pins
        .into_iter()
        .map(|(col, row, offset)| Pin { col, row, offset })
        .collect();
    Some(spec)
}

fn drive(
    keys: Res<ButtonInput<KeyCode>>,
    cursor: Res<CursorWorld>,
    level: Res<Level>,
    mut driven: Query<(&Driven, &Position, &mut LinearVelocity)>,
) {
    const SNAP: f32 = 20.0;
    for (d, pos, mut vel) in &mut driven {
        let key_dir = |up, down, left, right| {
            let axis = |pos: KeyCode, neg: KeyCode| {
                keys.pressed(pos) as i32 as f32 - keys.pressed(neg) as i32 as f32
            };
            Vec2::new(axis(right, left), axis(up, down)).normalize_or_zero()
        };
        let mut v = match d.control {
            Control::None => Vec2::ZERO,
            Control::Cursor => cursor
                .0
                .map(|c| ((c - pos.0) * SNAP).clamp_length_max(d.speed))
                .unwrap_or(Vec2::ZERO),
            Control::Arrows => {
                key_dir(
                    KeyCode::ArrowUp,
                    KeyCode::ArrowDown,
                    KeyCode::ArrowLeft,
                    KeyCode::ArrowRight,
                ) * d.speed
            }
            Control::Wasd => {
                key_dir(KeyCode::KeyW, KeyCode::KeyS, KeyCode::KeyA, KeyCode::KeyD) * d.speed
            }
        };
        match d.axis {
            Axis::Free => {}
            Axis::Horizontal => v.y = (d.home.y - pos.y) * SNAP,
            Axis::Vertical => v.x = (d.home.x - pos.x) * SNAP,
        }
        // Stay within range of home, and inside the level.
        let offset = pos.0 - d.home;
        if d.range > 0.0 && offset.length() >= d.range {
            let out = offset.normalize();
            v -= out * v.dot(out).max(0.0);
        }
        let b = level.bounds;
        if (pos.x >= b.x && v.x > 0.0) || (pos.x <= -b.x && v.x < 0.0) {
            v.x = 0.0;
        }
        if (pos.y >= b.y && v.y > 0.0) || (pos.y <= -b.y && v.y < 0.0) {
            v.y = 0.0;
        }
        vel.0 = v;
    }
}

/// Nudges each keep-speed element toward its target speed along its current direction of travel.
/// The same velocity change goes to every still-bonded body, so it never strains the element.
/// While the element touches anything it is left alone: pushing it then would crush it into
/// whatever it is hitting.
fn keep_speed(
    level: Res<Level>,
    time: Res<Time>,
    mut roots: Query<(Entity, &mut ElementRoot)>,
    mut bodies: Query<
        (
            &Group,
            &mut LinearVelocity,
            Option<&Cell>,
            Option<&CollidingEntities>,
        ),
        (With<RigidBody>, Without<Driven>),
    >,
    groups: Query<&Group>,
) {
    for (_, mut root) in &mut roots {
        root.age += time.delta_secs();
    }
    let targets: HashMap<Entity, (f32, bool)> = roots
        .iter()
        .filter(|(_, root)| !root.destroyed)
        .filter_map(|(entity, root)| {
            let e = level.elements.get(root.index)?;
            let target = e.keep_speed + e.speed_ramp * root.age;
            (e.keep_speed > 0.0).then_some((entity, (target, root.bonds > 0)))
        })
        .collect();
    if targets.is_empty() {
        return;
    }
    let counts = |cell: Option<&Cell>, bonded: bool| !bonded || cell.is_none_or(|c| c.bonds > 0);
    let mut sums: HashMap<Entity, (Vec2, f32, bool)> = HashMap::new();
    for (group, v, cell, colliding) in &bodies {
        let Some(&(_, bonded)) = targets.get(&group.0) else {
            continue;
        };
        if !counts(cell, bonded) {
            continue;
        }
        let touching = colliding.is_some_and(|c| {
            c.iter()
                .any(|other| groups.get(*other).map_or(true, |g| g.0 != group.0))
        });
        let sum = sums.entry(group.0).or_default();
        sum.0 += v.0;
        sum.1 += 1.0;
        sum.2 |= touching;
    }
    let rate = (3.0 * time.delta_secs()).min(1.0);
    let deltas: HashMap<Entity, Vec2> = sums
        .into_iter()
        .filter(|(_, (_, _, touching))| !touching)
        .filter_map(|(root, (sum, n, _))| {
            let (target, _) = targets[&root];
            let avg = sum / n;
            let speed = avg.length();
            (speed > 1.0).then(|| (root, avg / speed * (target - speed) * rate))
        })
        .collect();
    for (group, mut v, cell, _) in &mut bodies {
        if let (Some(delta), Some(&(_, bonded))) = (deltas.get(&group.0), targets.get(&group.0))
            && counts(cell, bonded)
        {
            v.0 += *delta;
        }
    }
}

fn track_elements(
    level: Res<Level>,
    mut roots: Query<(Entity, &mut ElementRoot, &HitTag)>,
    bonds: Query<(&Bond, &Group)>,
    bodies: Query<&Group, (With<RigidBody>, Without<Driven>)>,
    mut score: ResMut<Score>,
    mut highscores: ResMut<Highscores>,
    mut respawns: ResMut<Respawns>,
) {
    let mut intact: HashMap<Entity, usize> = HashMap::new();
    let mut intact_pins: HashMap<Entity, usize> = HashMap::new();
    for (bond, group) in &bonds {
        let counts = if bond.material.is_some() {
            &mut intact
        } else {
            &mut intact_pins
        };
        *counts.entry(group.0).or_default() += 1;
    }
    let mut alive: HashMap<Entity, usize> = HashMap::new();
    for group in &bodies {
        *alive.entry(group.0).or_default() += 1;
    }

    for (entity, mut root, tag) in &mut roots {
        if root.destroyed {
            continue;
        }
        let Some(element) = level.elements.get(root.index) else {
            continue;
        };
        if matches!(element.body, Body::Wall { .. }) {
            continue;
        }
        let lost = if root.bonds > 0 {
            1.0 - intact.get(&entity).copied().unwrap_or(0) as f32 / root.bonds as f32
        } else {
            0.0
        };
        let gone = alive.get(&entity).copied().unwrap_or(0) == 0;
        let shattered = lost > 0.0 && lost >= element.destroyed_at;
        let knocked_loose = root.pins > 0 && intact_pins.get(&entity).copied().unwrap_or(0) == 0;
        if !(gone || shattered || knocked_loose) {
            continue;
        }
        root.destroyed = true;
        let receiver = match element.credit {
            Credit::Player(player) => Some(player),
            Credit::LastHitter => tag.last_hit,
        };
        if element.points != 0
            && let Some(player) = receiver
        {
            score.points[player.index()] += element.points;
            let best = highscores.0.entry(level.name.clone()).or_insert(i32::MIN);
            *best = (*best).max(score.points[player.index()]);
        }
        if element.respawn {
            respawns.0.push((root.index, RESPAWN_DELAY));
        }
    }
}

fn respawn(
    mut commands: Commands,
    time: Res<Time>,
    level: Res<Level>,
    materials: Res<Materials>,
    anchor: Res<WorldAnchor>,
    mut respawns: ResMut<Respawns>,
) {
    let dt = time.delta_secs();
    respawns.0.retain_mut(|(index, delay)| {
        *delay -= dt;
        if *delay > 0.0 {
            return true;
        }
        if let Some(element) = level.elements.get(*index) {
            spawn_element(
                &mut commands,
                &materials,
                anchor.0,
                element,
                *index,
                level.resolution,
            );
        }
        false
    });
}

/// Blows up detonating cells: every dynamic body in reach gets an outward impulse that fades with
/// distance (so light debris flies further than heavy pieces), and the cell itself is gone.
/// Neighbouring explosive cells are shaken loose by this and go off on a later step, which makes
/// chain reactions spread through a charge instead of all at once.
fn detonate(
    mut commands: Commands,
    materials: Res<Materials>,
    detonating: Query<(Entity, &Position, &Cell), (With<Detonating>, Without<Doomed>)>,
    mut bodies: Query<
        (
            Entity,
            &Position,
            &ComputedMass,
            &mut LinearVelocity,
            Has<Sleeping>,
            Has<SweptCcd>,
        ),
        Without<Doomed>,
    >,
    mut explosions: ResMut<Explosions>,
) {
    explosions.0.clear();
    for (source, center, cell) in &detonating {
        let power = materials.get(cell.material).explosive;
        commands.entity(source).insert(Doomed);
        if power <= 0.0 {
            continue;
        }
        let radius = BLAST_RADIUS * power.sqrt();
        for (entity, pos, mass, mut velocity, asleep, has_ccd) in &mut bodies {
            let offset = pos.0 - center.0;
            let distance = offset.length();
            if entity == source || distance >= radius || mass.inverse() == 0.0 {
                continue;
            }
            let falloff = 1.0 - distance / radius;
            let direction = offset.try_normalize().unwrap_or(Vec2::Y);
            let pushed =
                velocity.0 + direction * (BLAST_IMPULSE * power * falloff * mass.inverse());
            velocity.0 = pushed.clamp_length_max(MAX_BLAST_SPEED.max(velocity.0.length()));
            if !has_ccd && velocity.0.length() > BLAST_CCD_SPEED {
                commands
                    .entity(entity)
                    .insert((SweptCcd::default(), BlastCcd(BLAST_CCD_SECONDS)));
            }
            // Only sleeping bodies belong to an island Avian can wake (others are awake anyway).
            if asleep {
                commands.queue(WakeBody(entity));
            }
        }
        explosions.0.push(Explosion {
            pos: center.0,
            radius,
            power,
        });
    }
}

/// Takes away the swept CCD a blast added, once the debris has slowed down to normal speeds.
fn expire_blast_ccd(
    mut commands: Commands,
    time: Res<Time>,
    mut blasted: Query<(Entity, &mut BlastCcd)>,
) {
    for (entity, mut ccd) in &mut blasted {
        ccd.0 -= time.delta_secs();
        if ccd.0 <= 0.0 {
            commands.entity(entity).remove::<(SweptCcd, BlastCcd)>();
        }
    }
}

/// Cells that lost all their bonds are debris: they fade out (see visuals) and are removed after
/// `DEBRIS_LIFETIME`. Cells that never had bonds (single-cell elements) are left alone.
fn age_debris(mut commands: Commands, time: Res<Time>, mut cells: Query<(Entity, &mut Cell)>) {
    let dt = time.delta_secs();
    for (entity, mut cell) in &mut cells {
        if cell.bonds > 0 {
            cell.was_bonded = true;
            cell.loose_for = 0.0;
        } else if cell.was_bonded {
            cell.loose_for += dt;
            if cell.loose_for > DEBRIS_LIFETIME {
                commands.entity(entity).insert(Doomed);
            }
        }
    }
}

fn cull_out_of_bounds(
    mut commands: Commands,
    level: Res<Level>,
    bodies: Query<(Entity, &Position, &RigidBody), (Without<Doomed>, Without<Driven>)>,
) {
    let b = level.bounds;
    for (entity, pos, body) in &bodies {
        if body.is_dynamic() && (pos.x.abs() > b.x || pos.y.abs() > b.y) {
            commands.entity(entity).insert(Doomed);
        }
    }
}

/// Set to remove loose cells (no intact bonds) and everything the gun fired.
#[derive(Resource, Default)]
pub struct ClearDebris(pub bool);

fn clear_debris(
    mut commands: Commands,
    mut request: ResMut<ClearDebris>,
    cells: Query<(Entity, &Cell)>,
    projectiles: Query<Entity, With<Projectile>>,
    members: Query<(Entity, &Group)>,
) {
    if !std::mem::take(&mut request.0) {
        return;
    }
    for (entity, cell) in &cells {
        if cell.bonds == 0 {
            commands.entity(entity).insert(Doomed);
        }
    }
    for entity in &projectiles {
        crate::lattice::doom_group(&mut commands, entity, &members);
    }
}
