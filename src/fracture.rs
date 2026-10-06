//! Elastoplastic bonds: each physics step, measure how far every bond's two anchor frames have
//! been pulled apart. Elastic strain springs back on its own (it's just XPBD compliance), strain
//! beyond yield is absorbed into the rest frames (permanent bending) and accumulated as damage,
//! and a bond snaps when its damage exceeds the material's ductility or its strain the break limit.

use std::f32::consts::{PI, TAU};

use avian2d::prelude::*;
use bevy::prelude::*;

use crate::lattice::{Bond, Cell, Detonating};
use crate::materials::Materials;

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

pub fn plasticity(
    mut commands: Commands,
    materials: Res<Materials>,
    time: Res<Time<Virtual>>,
    mut joints: Query<(Entity, &mut FixedJoint, &mut Bond)>,
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
        let mut frame_changed = false;
        let (mut a1, mut a2, mut b1, mut b2) = (a1, a2, b1, b2);
        if strain > s.yield_strain {
            // Move both anchors toward each other until only the yield strain remains.
            let excess = gap * (1.0 - s.yield_strain / strain);
            a1 += r1.inverse() * (excess * 0.5);
            a2 -= r2.inverse() * (excess * 0.5);
            plastic += excess.length() / size;
            frame_changed = true;
        }
        if angle.abs() > s.yield_angle {
            let excess = angle - angle.signum() * s.yield_angle;
            b1 = Rotation::radians(b1.as_radians() + excess * 0.5);
            b2 = Rotation::radians(b2.as_radians() - excess * 0.5);
            plastic += excess.abs();
            frame_changed = true;
        }
        bond.damage += plastic;
        stats.plastic += plastic;

        let broken = strain > s.break_strain
            || angle.abs() > s.break_angle
            || (plastic > 0.0 && bond.damage > s.ductility);

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
        for body in [joint.body1, joint.body2] {
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
            commands.entity(entity).despawn();
            stats.broken += 1;
            breaks.0.push(Break {
                pos: (w1 + w2) * 0.5,
                time: now,
                color,
            });
            continue;
        }
        stats.bonds += 1;
        if frame_changed {
            joint.frame1.anchor = JointAnchor::Local(a1);
            joint.frame2.anchor = JointAnchor::Local(a2);
            joint.frame1.basis = JointBasis::Local(b1);
            joint.frame2.basis = JointBasis::Local(b2);
        }
    }

    for mut cell in &mut cells {
        let base = cell.stress_base;
        cell.stress_base = base + (cell.stress - base) * STRESS_BASE_RATE;
    }
}
