//! Destructible bodies are grids of small rigid cells glued together by compliant fixed joints
//! ("bonds"). Structures are fixed to an anchor body (the static world, or a kinematic carrier
//! that the player drives) by the same kind of joint ("pins").
//! The joints are solved by Avian's XPBD solver, so an impact on one cell propagates through the
//! bond network; `fracture.rs` makes the bonds yield plastically and snap.

use avian2d::prelude::*;
use bevy::prelude::*;

use crate::materials::{MaterialKind, Materials, Strength};

/// Cell colliders are slightly smaller than their cell so neighbours don't rub at rest.
const COLLIDER_SCALE: f32 = 0.92;

#[derive(Component)]
pub struct Cell {
    pub material: MaterialKind,
    /// 0..1, how close the most damaged bond attached to this cell ever came to failing.
    pub damage: f32,
    /// Intact bonds and pins attached to this cell, recounted every physics step.
    pub bonds: u8,
    /// Whether this cell has ever been bonded; only those count as debris once loose.
    pub was_bonded: bool,
    /// Seconds since the cell lost its last bond.
    pub loose_for: f32,
    /// Highest load on its bonds this physics step, relative to their break limits.
    pub stress: f32,
    /// Slowly following average of `stress`: the steady load (e.g. from gravity). The stress
    /// glow shows `stress` above this, i.e. impacts travelling through the structure.
    pub stress_base: f32,
}

/// An explosive cell that lost a bond; it blows up this frame.
#[derive(Component)]
pub struct Detonating;

/// A bond between two cells, or a pin (`material == None`) between an anchor and a body.
#[derive(Component)]
pub struct Bond {
    pub material: Option<MaterialKind>,
    pub cell_size: f32,
    /// Accumulated plastic deformation.
    pub damage: f32,
    /// Current elastic load relative to the instant-break limit, for the stress overlay.
    pub strain: f32,
}

impl Bond {
    pub fn pin(cell_size: f32) -> Self {
        Self {
            material: None,
            cell_size,
            damage: 0.0,
            strain: 0.0,
        }
    }

    pub fn strength(&self, materials: &Materials) -> Strength {
        match self.material {
            Some(kind) => materials.get(kind).strength,
            None => materials.pins,
        }
    }
}

/// Root entity of one spawned element or projectile.
#[derive(Component)]
pub struct GroupRoot;

/// Points a body or joint at its `GroupRoot`, so whole elements can be tracked and removed.
#[derive(Component, Clone, Copy)]
pub struct Group(pub Entity);

/// Static body at the origin that world-fixed pins attach to.
#[derive(Resource)]
pub struct WorldAnchor(pub Entity);

/// Marks an entity for removal. Joints touching a doomed body are removed with it.
#[derive(Component)]
pub struct Doomed;

pub struct Pin {
    pub col: i32,
    pub row: i32,
    /// Where on the cell the pin sits, in the cell's local frame.
    pub offset: Vec2,
}

pub struct LatticeSpec {
    /// World position of the lattice's center.
    pub center: Vec2,
    pub angle: f32,
    pub cols: i32,
    pub rows: i32,
    pub cell: f32,
    /// Length that bond strain is measured against: the element's own cell size, even when the
    /// level's physics resolution divides it into smaller cells. Measured against the small
    /// cells, the same physical movement would count as more strain, and finer lattices would
    /// be weaker; this keeps materials equally strong at every resolution.
    pub strain_length: f32,
    pub material: MaterialKind,
    pub velocity: Vec2,
    /// Only keep the cells inside the inscribed ellipse.
    pub round: bool,
    pub pins: Vec<Pin>,
    pub ccd: bool,
    /// Let Avian put the cells to sleep once the lattice is at rest. Off for lattices that hang
    /// from a moving carrier or get steered, since sleeping cells would stop following.
    pub can_sleep: bool,
    /// Overrides the material's restitution, combined with `Max` so it wins against anything.
    pub bounce: Option<f32>,
    /// Overrides the material's friction, combined with `Min` so it wins against anything.
    pub friction: Option<f32>,
}

impl LatticeSpec {
    pub fn has_cell(&self, i: i32, j: i32) -> bool {
        if i < 0 || j < 0 || i >= self.cols || j >= self.rows {
            return false;
        }
        if !self.round {
            return true;
        }
        let x = (i as f32 - (self.cols - 1) as f32 * 0.5) / (self.cols as f32 * 0.5);
        let y = (j as f32 - (self.rows - 1) as f32 * 0.5) / (self.rows as f32 * 0.5);
        x * x + y * y <= 1.0
    }

    /// Position of cell (i, j) relative to the lattice center, before rotation.
    pub fn cell_local(&self, i: i32, j: i32) -> Vec2 {
        Vec2::new(
            i as f32 - (self.cols - 1) as f32 * 0.5,
            j as f32 - (self.rows - 1) as f32 * 0.5,
        ) * self.cell
    }
}

/// Spawns the cells, bonds and pins of a lattice as members of `root`. Pins attach to `anchor`,
/// whose current pose is `anchor_pose`. Returns the number of internal bonds and pins created.
pub fn spawn_lattice(
    commands: &mut Commands,
    materials: &Materials,
    anchor: Entity,
    anchor_pose: Isometry2d,
    spec: &LatticeSpec,
    root: Entity,
) -> (usize, usize) {
    let group = Group(root);
    let params = materials.get(spec.material);
    let strength = params.strength;
    let half = spec.cell * 0.5;
    let rotation = Rot2::radians(spec.angle);
    let restitution = match spec.bounce {
        Some(bounce) => Restitution::new(bounce).with_combine_rule(CoefficientCombine::Max),
        None => Restitution::new(params.restitution),
    };
    let friction = match spec.friction {
        Some(f) => Friction::new(f).with_combine_rule(CoefficientCombine::Min),
        None => Friction::new(params.friction),
    };

    let mut cells = vec![None; (spec.cols * spec.rows) as usize];
    for j in 0..spec.rows {
        for i in 0..spec.cols {
            if !spec.has_cell(i, j) {
                continue;
            }
            let pos = spec.center + rotation * spec.cell_local(i, j);
            let mut cell = commands.spawn((
                Cell {
                    material: spec.material,
                    damage: 0.0,
                    bonds: 0,
                    was_bonded: false,
                    loose_for: 0.0,
                    stress: 0.0,
                    stress_base: 0.0,
                },
                group,
                RigidBody::Dynamic,
                Collider::rectangle(spec.cell * COLLIDER_SCALE, spec.cell * COLLIDER_SCALE),
                ColliderDensity(params.density / (COLLIDER_SCALE * COLLIDER_SCALE)),
                friction,
                restitution,
                LinearVelocity(spec.velocity),
                Transform::from_translation(pos.extend(0.0))
                    .with_rotation(Quat::from_rotation_z(spec.angle)),
                Sprite::from_color(params.color, Vec2::splat(spec.cell)),
            ));
            if spec.ccd {
                cell.insert(SweptCcd::default());
            }
            if !spec.can_sleep {
                cell.insert(SleepingDisabled);
            }
            cells[(j * spec.cols + i) as usize] = Some(cell.id());
        }
    }

    let at = |i: i32, j: i32| {
        if spec.has_cell(i, j) {
            cells[(j * spec.cols + i) as usize]
        } else {
            None
        }
    };

    let mut bonds = 0;
    for j in 0..spec.rows {
        for i in 0..spec.cols {
            let Some(a) = at(i, j) else { continue };
            for (di, dj) in [(1, 0), (0, 1)] {
                let Some(b) = at(i + di, j + dj) else {
                    continue;
                };
                let dir = Vec2::new(di as f32, dj as f32);
                commands.spawn((
                    FixedJoint::new(a, b)
                        .with_local_anchor1(dir * half)
                        .with_local_anchor2(-dir * half)
                        .with_point_compliance(strength.point_compliance)
                        .with_angle_compliance(strength.angle_compliance),
                    JointCollisionDisabled,
                    Bond {
                        material: Some(spec.material),
                        cell_size: spec.strain_length,
                        damage: 0.0,
                        strain: 0.0,
                    },
                    group,
                ));
                bonds += 1;
            }
        }
    }

    let mut pins = 0;
    for pin in &spec.pins {
        let Some(cell) = at(pin.col, pin.row) else {
            continue;
        };
        pins += 1;
        let world = spec.center + rotation * (spec.cell_local(pin.col, pin.row) + pin.offset);
        spawn_pin(
            commands,
            materials,
            anchor,
            anchor_pose,
            cell,
            Isometry2d::new(world, rotation),
            pin.offset,
            spec.strain_length,
            group,
        );
    }

    (bonds, pins)
}

/// Pins `body` to `anchor` at the world point of `at`. `body_offset` is that point in the body's
/// local frame, and `at.rotation` the body's current rotation.
pub fn spawn_pin(
    commands: &mut Commands,
    materials: &Materials,
    anchor: Entity,
    anchor_pose: Isometry2d,
    body: Entity,
    at: Isometry2d,
    body_offset: Vec2,
    cell_size: f32,
    group: Group,
) {
    let local = anchor_pose.inverse() * at;
    commands.spawn((
        FixedJoint::new(anchor, body)
            .with_local_anchor1(local.translation)
            .with_local_anchor2(body_offset)
            // The bodies are at rest in this relative orientation.
            .with_local_basis1(local.rotation.as_radians())
            .with_point_compliance(materials.pins.point_compliance)
            .with_angle_compliance(materials.pins.angle_compliance),
        Bond::pin(cell_size),
        group,
    ));
}

/// Removes doomed entities, plus any joint attached to a doomed body so no joint dangles.
pub fn despawn_doomed(
    mut commands: Commands,
    doomed: Query<Entity, With<Doomed>>,
    joints: Query<(Entity, &FixedJoint), Without<Doomed>>,
) {
    if doomed.is_empty() {
        return;
    }
    for (entity, joint) in &joints {
        if doomed.contains(joint.body1) || doomed.contains(joint.body2) {
            commands.entity(entity).despawn();
        }
    }
    for entity in &doomed {
        commands.entity(entity).despawn();
    }
}

/// Marks a whole group (root, bodies and joints) for removal.
pub fn doom_group(commands: &mut Commands, root: Entity, members: &Query<(Entity, &Group)>) {
    commands.entity(root).insert(Doomed);
    for (entity, group) in members {
        if group.0 == root {
            commands.entity(entity).insert(Doomed);
        }
    }
}

/// Keeps joint compliance in sync with the live-edited material table.
pub fn sync_compliance(materials: Res<Materials>, mut joints: Query<(&mut FixedJoint, &Bond)>) {
    if !materials.is_changed() {
        return;
    }
    for (mut joint, bond) in &mut joints {
        let s = bond.strength(&materials);
        joint.point_compliance = s.point_compliance;
        joint.angle_compliance = s.angle_compliance;
    }
}
