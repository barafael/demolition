//! Elastoplastic bonds: each physics step, measure how far every bond's two anchor frames have
//! been pulled apart. Elastic strain springs back on its own (it's just XPBD compliance), strain
//! beyond yield is absorbed into the rest frames (permanent bending) and accumulated as damage,
//! and a bond snaps when its damage exceeds the material's ductility or its strain the break limit.

use std::f32::consts::{PI, TAU};

use avian2d::prelude::*;
use bevy::prelude::*;

use crate::lattice::{Bond, Cell, Detonating};
use crate::materials::{Materials, Strength};

#[derive(Resource, Default)]
pub struct Stats {
    pub bonds: usize,
    pub broken: usize,
    pub plastic: f32,
}

/// A bond failure, for effects at the place it happened.
#[derive(Clone, Copy)]
pub struct Break {
    pub pos: Vec2,
    /// Virtual time of the break.
    pub time: f32,
    /// Color of the material that broke (pins count as steel-grey).
    pub color: Color,
}

/// Recent bond failures. Effects read them; `draw_breaks` prunes old ones.
#[derive(Resource, Default)]
pub struct Breaks(pub Vec<Break>);

/// How fast a cell's stress baseline follows its stress, per physics step.
const STRESS_BASE_RATE: f32 = 0.03;

fn wrap_angle(a: f32) -> f32 {
    (a + PI).rem_euclid(TAU) - PI
}

/// What a bond's measurement step found, for `settle`.
struct Measured {
    entity: Entity,
    bodies: [Entity; 2],
    /// Where the bond is, for break effects.
    at: Vec2,
    /// Plastic deformation absorbed this step.
    plastic: f32,
    /// Strain or angle beyond the instant-break limit.
    over_limit: bool,
}

/// Shared bookkeeping for every kind of bond and pin: plastic damage, the cells' wear, load
/// and bond count, and breaking (with its effects and detonations). Returns whether it broke.
fn settle(
    m: Measured,
    bond: &mut Bond,
    s: &Strength,
    materials: &Materials,
    now: f32,
    commands: &mut Commands,
    cells: &mut Query<&mut Cell>,
    stats: &mut Stats,
    breaks: &mut Breaks,
) -> bool {
    bond.damage += m.plastic;
    stats.plastic += m.plastic;
    let broken = m.over_limit || (m.plastic > 0.0 && bond.damage > s.ductility);

    let wear = if s.ductility > 0.0 {
        bond.damage / s.ductility
    } else {
        0.0
    };
    let wear = if broken {
        1.0
    } else {
        wear.max(bond.strain * 0.5).min(1.0)
    };
    let mut color = Color::srgb(0.55, 0.57, 0.6);
    for body in m.bodies {
        if let Ok(mut cell) = cells.get_mut(body) {
            cell.damage = cell.damage.max(wear * 0.8);
            cell.stress = cell.stress.max(bond.strain);
            if !broken {
                cell.bonds += 1;
                continue;
            }
            let params = materials.get(cell.material);
            color = params.color;
            if params.explosive > 0.0 {
                commands.entity(body).insert(Detonating);
            }
        }
    }

    if broken {
        commands.entity(m.entity).despawn();
        stats.broken += 1;
        breaks.0.push(Break {
            pos: m.at,
            time: now,
            color,
        });
    } else {
        stats.bonds += 1;
    }
    broken
}

/// Pulls two anchor points back together until only the yield strain remains, as permanent
/// deformation. Returns the new local anchors and the plastic strain absorbed.
fn yield_anchors(
    gap: Vec2,
    strain: f32,
    s: &Strength,
    size: f32,
    (a1, r1): (Vec2, &Rotation),
    (a2, r2): (Vec2, &Rotation),
) -> Option<(Vec2, Vec2, f32)> {
    if strain <= s.yield_strain {
        return None;
    }
    let excess = gap * (1.0 - s.yield_strain / strain);
    Some((
        a1 + r1.inverse() * (excess * 0.5),
        a2 - r2.inverse() * (excess * 0.5),
        excess.length() / size,
    ))
}

pub fn plasticity(
    mut commands: Commands,
    materials: Res<Materials>,
    time: Res<Time<Virtual>>,
    mut joints: Query<
        (Entity, &mut FixedJoint, &mut Bond),
        (Without<RevoluteJoint>, Without<DistanceJoint>),
    >,
    mut hinges: Query<
        (Entity, &mut RevoluteJoint, &mut Bond),
        (Without<FixedJoint>, Without<DistanceJoint>),
    >,
    mut ropes: Query<
        (Entity, &mut DistanceJoint, &mut Bond),
        (Without<FixedJoint>, Without<RevoluteJoint>),
    >,
    bodies: Query<(&Position, &Rotation)>,
    mut cells: Query<&mut Cell>,
    mut stats: ResMut<Stats>,
    mut breaks: ResMut<Breaks>,
) {
    for mut cell in &mut cells {
        cell.bonds = 0;
        cell.stress = 0.0;
    }
    stats.bonds = 0;
    let now = time.elapsed_secs();

    // Bonds and rigid pins: position and angle.
    for (entity, mut joint, mut bond) in &mut joints {
        let Ok([(p1, r1), (p2, r2)]) = bodies.get_many([joint.body1, joint.body2]) else {
            continue;
        };
        let (JointAnchor::Local(a1), JointAnchor::Local(a2)) =
            (joint.frame1.anchor, joint.frame2.anchor)
        else {
            continue;
        };
        let (JointBasis::Local(b1), JointBasis::Local(b2)) =
            (joint.frame1.basis, joint.frame2.basis)
        else {
            continue;
        };
        let s = bond.strength(&materials);
        let size = bond.cell_size;

        let w1 = p1.0 + *r1 * a1;
        let w2 = p2.0 + *r2 * a2;
        let gap = w2 - w1;
        let strain = gap.length() / size;
        let angle =
            wrap_angle(r2.as_radians() + b2.as_radians() - r1.as_radians() - b1.as_radians());
        bond.strain = (strain / s.break_strain).max(angle.abs() / s.break_angle);

        let mut plastic = 0.0;
        let (mut a1, mut a2, mut b1, mut b2) = (a1, a2, b1, b2);
        let mut frame_changed = false;
        if let Some((n1, n2, p)) = yield_anchors(gap, strain, &s, size, (a1, r1), (a2, r2)) {
            (a1, a2) = (n1, n2);
            plastic += p;
            frame_changed = true;
        }
        if angle.abs() > s.yield_angle {
            let excess = angle - angle.signum() * s.yield_angle;
            b1 = Rotation::radians(b1.as_radians() + excess * 0.5);
            b2 = Rotation::radians(b2.as_radians() - excess * 0.5);
            plastic += excess.abs();
            frame_changed = true;
        }
        let measured = Measured {
            entity,
            bodies: [joint.body1, joint.body2],
            at: (w1 + w2) * 0.5,
            plastic,
            over_limit: strain > s.break_strain || angle.abs() > s.break_angle,
        };
        let broken = settle(
            measured,
            &mut bond,
            &s,
            &materials,
            now,
            &mut commands,
            &mut cells,
            &mut stats,
            &mut breaks,
        );
        if !broken && frame_changed {
            joint.frame1.anchor = JointAnchor::Local(a1);
            joint.frame2.anchor = JointAnchor::Local(a2);
            joint.frame1.basis = JointBasis::Local(b1);
            joint.frame2.basis = JointBasis::Local(b2);
        }
    }

    // Hinge pins: position only; the body turns freely.
    for (entity, mut joint, mut bond) in &mut hinges {
        let Ok([(p1, r1), (p2, r2)]) = bodies.get_many([joint.body1, joint.body2]) else {
            continue;
        };
        let (JointAnchor::Local(a1), JointAnchor::Local(a2)) =
            (joint.frame1.anchor, joint.frame2.anchor)
        else {
            continue;
        };
        let s = bond.strength(&materials);
        let size = bond.cell_size;
        let (w1, w2) = (p1.0 + *r1 * a1, p2.0 + *r2 * a2);
        let gap = w2 - w1;
        let strain = gap.length() / size;
        bond.strain = strain / s.break_strain;
        let yielded = yield_anchors(gap, strain, &s, size, (a1, r1), (a2, r2));
        let measured = Measured {
            entity,
            bodies: [joint.body1, joint.body2],
            at: (w1 + w2) * 0.5,
            plastic: yielded.map_or(0.0, |(_, _, p)| p),
            over_limit: strain > s.break_strain,
        };
        let broken = settle(
            measured,
            &mut bond,
            &s,
            &materials,
            now,
            &mut commands,
            &mut cells,
            &mut stats,
            &mut breaks,
        );
        if !broken && let Some((n1, n2, _)) = yielded {
            joint.frame1.anchor = JointAnchor::Local(n1);
            joint.frame2.anchor = JointAnchor::Local(n2);
        }
    }

    // Rope pins: only stretching beyond the rope's length counts; a yielding rope gets longer.
    for (entity, mut joint, mut bond) in &mut ropes {
        let Ok([(p1, r1), (p2, r2)]) = bodies.get_many([joint.body1, joint.body2]) else {
            continue;
        };
        let (JointAnchor::Local(a1), JointAnchor::Local(a2)) = (joint.anchor1, joint.anchor2)
        else {
            continue;
        };
        let s = bond.strength(&materials);
        let size = bond.cell_size;
        let (w1, w2) = (p1.0 + *r1 * a1, p2.0 + *r2 * a2);
        let stretch = ((w2 - w1).length() - joint.limits.max).max(0.0);
        let strain = stretch / size;
        bond.strain = strain / s.break_strain;
        let plastic = (strain - s.yield_strain).max(0.0);
        let measured = Measured {
            entity,
            bodies: [joint.body1, joint.body2],
            at: w2,
            plastic,
            over_limit: strain > s.break_strain,
        };
        let broken = settle(
            measured,
            &mut bond,
            &s,
            &materials,
            now,
            &mut commands,
            &mut cells,
            &mut stats,
            &mut breaks,
        );
        if !broken && plastic > 0.0 {
            joint.limits.max += plastic * size;
        }
    }

    for mut cell in &mut cells {
        let base = cell.stress_base;
        cell.stress_base = base + (cell.stress - base) * STRESS_BASE_RATE;
    }

    // The effects take the whole queue every frame; headless runs have no taker, so keep the
    // log bounded there.
    if breaks.0.len() > 8192 {
        breaks.0.drain(..4096);
    }
}
