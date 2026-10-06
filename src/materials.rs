use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// How a bond (or pin) responds to strain. Strains are measured in cell sizes, angles in radians.
///
/// - Below `yield_*` the bond is elastic and springs back.
/// - Beyond yield its rest pose drifts toward the current pose (it stays bent), and the drift is
///   accumulated as plastic damage.
/// - It snaps once the damage exceeds `ductility`, or instantly if the strain exceeds `break_*`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Strength {
    /// XPBD compliance (inverse stiffness) of the anchor-point constraint.
    pub point_compliance: f32,
    /// XPBD compliance of the relative-angle constraint.
    pub angle_compliance: f32,
    pub yield_strain: f32,
    pub yield_angle: f32,
    /// Accumulated plastic deformation (cell sizes + radians) the bond survives.
    pub ductility: f32,
    pub break_strain: f32,
    pub break_angle: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MaterialParams {
    pub color: Color,
    /// Mass per square pixel.
    pub density: f32,
    pub friction: f32,
    pub restitution: f32,
    pub strength: Strength,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MaterialKind {
    Glass,
    Wood,
    Concrete,
    Steel,
    Rubber,
    Clay,
}

impl MaterialKind {
    pub const ALL: [MaterialKind; 6] = [
        MaterialKind::Glass,
        MaterialKind::Wood,
        MaterialKind::Concrete,
        MaterialKind::Steel,
        MaterialKind::Rubber,
        MaterialKind::Clay,
    ];

    pub fn name(self) -> &'static str {
        match self {
            MaterialKind::Glass => "Glass",
            MaterialKind::Wood => "Wood",
            MaterialKind::Concrete => "Concrete",
            MaterialKind::Steel => "Steel",
            MaterialKind::Rubber => "Rubber",
            MaterialKind::Clay => "Clay",
        }
    }
}

/// Live-tunable material table. Systems read it every step, so edits apply immediately.
/// Saved with each level, so a level can define its own material behaviour.
#[derive(Resource, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Materials {
    pub table: [MaterialParams; 6],
    /// The bolts that fix structures to the world.
    pub pins: Strength,
}

impl Materials {
    pub fn get(&self, kind: MaterialKind) -> &MaterialParams {
        &self.table[kind as usize]
    }
}

impl Default for Materials {
    fn default() -> Self {
        Self {
            table: [
                MaterialParams {
                    color: Color::srgb(0.62, 0.85, 0.95),
                    density: 0.025,
                    friction: 0.2,
                    restitution: 0.1,
                    strength: Strength {
                        point_compliance: 1e-7,
                        angle_compliance: 1e-7,
                        yield_strain: 0.07,
                        yield_angle: 0.1,
                        ductility: 0.0,
                        break_strain: 0.07,
                        break_angle: 0.1,
                    },
                },
                MaterialParams {
                    color: Color::srgb(0.66, 0.46, 0.26),
                    density: 0.006,
                    friction: 0.6,
                    restitution: 0.2,
                    strength: Strength {
                        point_compliance: 4e-7,
                        angle_compliance: 4e-7,
                        yield_strain: 0.1,
                        yield_angle: 0.14,
                        ductility: 0.4,
                        break_strain: 0.3,
                        break_angle: 0.45,
                    },
                },
                MaterialParams {
                    color: Color::srgb(0.6, 0.6, 0.58),
                    density: 0.024,
                    friction: 0.8,
                    restitution: 0.05,
                    strength: Strength {
                        point_compliance: 1e-7,
                        angle_compliance: 1e-7,
                        yield_strain: 0.06,
                        yield_angle: 0.09,
                        ductility: 0.06,
                        break_strain: 0.1,
                        break_angle: 0.14,
                    },
                },
                MaterialParams {
                    color: Color::srgb(0.45, 0.5, 0.6),
                    density: 0.078,
                    friction: 0.4,
                    restitution: 0.2,
                    strength: Strength {
                        point_compliance: 5e-8,
                        angle_compliance: 5e-8,
                        yield_strain: 0.06,
                        yield_angle: 0.08,
                        ductility: 3.0,
                        break_strain: 0.8,
                        break_angle: 1.4,
                    },
                },
                MaterialParams {
                    color: Color::srgb(0.85, 0.3, 0.35),
                    density: 0.011,
                    friction: 0.9,
                    restitution: 0.8,
                    strength: Strength {
                        point_compliance: 1.5e-6,
                        angle_compliance: 1.5e-6,
                        yield_strain: 1.0,
                        yield_angle: 1.5,
                        ductility: 0.5,
                        break_strain: 1.2,
                        break_angle: 2.0,
                    },
                },
                MaterialParams {
                    color: Color::srgb(0.75, 0.55, 0.4),
                    density: 0.018,
                    friction: 0.9,
                    restitution: 0.0,
                    strength: Strength {
                        point_compliance: 1e-6,
                        angle_compliance: 1e-6,
                        yield_strain: 0.08,
                        yield_angle: 0.12,
                        ductility: 12.0,
                        break_strain: 1.2,
                        break_angle: 2.5,
                    },
                },
            ],
            pins: Strength {
                point_compliance: 1e-7,
                angle_compliance: 1e-7,
                yield_strain: 0.1,
                yield_angle: 0.12,
                ductility: 1.5,
                break_strain: 0.6,
                break_angle: 1.2,
            },
        }
    }
}
