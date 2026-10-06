use bevy::prelude::*;
use serde::{Deserialize, Deserializer, Serialize};

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
    /// How hard a cell of this material blows up when it loses its first bond (0 = inert).
    #[serde(default)]
    pub explosive: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MaterialKind {
    Glass,
    Wood,
    Concrete,
    Steel,
    Rubber,
    Clay,
    Ice,
    Jelly,
    Honey,
    Tnt,
}

pub const MATERIAL_COUNT: usize = 10;

impl MaterialKind {
    pub const ALL: [MaterialKind; MATERIAL_COUNT] = [
        MaterialKind::Glass,
        MaterialKind::Wood,
        MaterialKind::Concrete,
        MaterialKind::Steel,
        MaterialKind::Rubber,
        MaterialKind::Clay,
        MaterialKind::Ice,
        MaterialKind::Jelly,
        MaterialKind::Honey,
        MaterialKind::Tnt,
    ];

    pub fn name(self) -> &'static str {
        match self {
            MaterialKind::Glass => "Glass",
            MaterialKind::Wood => "Wood",
            MaterialKind::Concrete => "Concrete",
            MaterialKind::Steel => "Steel",
            MaterialKind::Rubber => "Rubber",
            MaterialKind::Clay => "Clay",
            MaterialKind::Ice => "Ice",
            MaterialKind::Jelly => "Jelly",
            MaterialKind::Honey => "Honey",
            MaterialKind::Tnt => "TNT",
        }
    }
}

/// Live-tunable material table. Systems read it every step, so edits apply immediately.
/// Saved with each level, so a level can define its own material behaviour.
#[derive(Resource, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Materials {
    #[serde(deserialize_with = "deserialize_table")]
    pub table: [MaterialParams; MATERIAL_COUNT],
    /// The bolts that fix structures to the world.
    pub pins: Strength,
}

/// Reads a material table that may predate some materials: missing entries keep their
/// defaults, so levels saved before a material was added still load.
fn deserialize_table<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<[MaterialParams; MATERIAL_COUNT], D::Error> {
    struct Table;
    impl<'de> serde::de::Visitor<'de> for Table {
        type Value = [MaterialParams; MATERIAL_COUNT];

        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a list of material parameters")
        }

        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> Result<Self::Value, A::Error> {
            let mut table = Materials::default().table;
            for slot in &mut table {
                match seq.next_element()? {
                    Some(params) => *slot = params,
                    None => break,
                }
            }
            // Ignore materials from a newer version this one doesn't know.
            while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {}
            Ok(table)
        }
    }
    deserializer.deserialize_tuple(MATERIAL_COUNT, Table)
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
                    explosive: 0.0,
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
                    explosive: 0.0,
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
                    explosive: 0.0,
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
                    explosive: 0.0,
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
                    explosive: 0.0,
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
                    explosive: 0.0,
                },
                // Ice: glassy and brittle, but nearly frictionless.
                MaterialParams {
                    color: Color::srgb(0.86, 0.95, 1.0),
                    density: 0.009,
                    friction: 0.02,
                    restitution: 0.1,
                    strength: Strength {
                        point_compliance: 1e-7,
                        angle_compliance: 1e-7,
                        yield_strain: 0.06,
                        yield_angle: 0.09,
                        ductility: 0.0,
                        break_strain: 0.06,
                        break_angle: 0.09,
                    },
                    explosive: 0.0,
                },
                // Jelly: very soft and bouncy; never yields, never breaks.
                MaterialParams {
                    color: Color::srgb(0.5, 0.88, 0.45),
                    density: 0.010,
                    friction: 0.6,
                    restitution: 0.9,
                    strength: Strength {
                        point_compliance: 5e-6,
                        angle_compliance: 5e-6,
                        yield_strain: 10.0,
                        yield_angle: 10.0,
                        ductility: 1.0,
                        break_strain: 10.0,
                        break_angle: 10.0,
                    },
                    explosive: 0.0,
                },
                // Honey: soft and sticky, yields almost at once so it flows and sags, and only
                // drips apart after a lot of flowing.
                MaterialParams {
                    color: Color::srgb(0.95, 0.68, 0.12),
                    density: 0.014,
                    friction: 1.2,
                    restitution: 0.0,
                    strength: Strength {
                        point_compliance: 3e-6,
                        angle_compliance: 3e-6,
                        yield_strain: 0.02,
                        yield_angle: 0.03,
                        ductility: 30.0,
                        break_strain: 1.5,
                        break_angle: 3.0,
                    },
                    explosive: 0.0,
                },
                // TNT: a weak solid that blows up when it starts to break.
                MaterialParams {
                    color: Color::srgb(0.78, 0.16, 0.12),
                    density: 0.016,
                    friction: 0.6,
                    restitution: 0.1,
                    strength: Strength {
                        point_compliance: 4e-7,
                        angle_compliance: 4e-7,
                        yield_strain: 0.08,
                        yield_angle: 0.1,
                        ductility: 0.1,
                        break_strain: 0.15,
                        break_angle: 0.2,
                    },
                    explosive: 1.0,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_from_before_new_materials_still_load() {
        /// The shape of the material table when there were six materials.
        #[derive(Serialize)]
        struct Legacy {
            table: [MaterialParams; 6],
            pins: Strength,
        }
        let defaults = Materials::default();
        let legacy = Legacy {
            table: std::array::from_fn(|i| defaults.table[i].clone()),
            pins: defaults.pins,
        };
        let loaded: Materials = ron::from_str(&ron::to_string(&legacy).unwrap()).unwrap();
        assert_eq!(loaded, defaults);
    }
}
