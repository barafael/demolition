use avian2d::prelude::*;
use bevy::prelude::*;

use crate::lattice::{Doomed, Group, GroupRoot, LatticeSpec, doom_group, spawn_lattice};
use crate::level::Player;
use crate::materials::{MaterialKind, Materials};
use crate::play::{HitTag, LevelEntity, ReportHits};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ammo {
    Pebble,
    Cannonball,
    RubberBall,
    Bullet,
    ClaySlug,
    GlassMarble,
}

pub enum AmmoShape {
    Ball {
        radius: f32,
        density: f32,
        restitution: f32,
        friction: f32,
        color: Color,
    },
    /// A small destructible lattice, so the projectile itself deforms and breaks.
    Cluster {
        material: MaterialKind,
        radius_cells: i32,
        cell: f32,
    },
}

impl Ammo {
    pub const ALL: [Ammo; 6] = [
        Ammo::Pebble,
        Ammo::Cannonball,
        Ammo::RubberBall,
        Ammo::Bullet,
        Ammo::ClaySlug,
        Ammo::GlassMarble,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Ammo::Pebble => "Pebble",
            Ammo::Cannonball => "Cannonball",
            Ammo::RubberBall => "Rubber ball",
            Ammo::Bullet => "Bullet",
            Ammo::ClaySlug => "Clay slug",
            Ammo::GlassMarble => "Glass marble",
        }
    }

    pub fn default_speed(self) -> f32 {
        match self {
            Ammo::Pebble => 1200.0,
            Ammo::Cannonball => 900.0,
            Ammo::RubberBall => 1100.0,
            Ammo::Bullet => 3000.0,
            Ammo::ClaySlug => 1000.0,
            Ammo::GlassMarble => 1000.0,
        }
    }

    pub fn shape(self) -> AmmoShape {
        match self {
            Ammo::Pebble => AmmoShape::Ball {
                radius: 5.0,
                density: 0.027,
                restitution: 0.3,
                friction: 0.6,
                color: Color::srgb(0.55, 0.52, 0.48),
            },
            Ammo::Cannonball => AmmoShape::Ball {
                radius: 16.0,
                density: 0.078,
                restitution: 0.1,
                friction: 0.4,
                color: Color::srgb(0.55, 0.57, 0.62),
            },
            Ammo::RubberBall => AmmoShape::Ball {
                radius: 12.0,
                density: 0.011,
                restitution: 0.9,
                friction: 0.9,
                color: Color::srgb(0.95, 0.35, 0.6),
            },
            Ammo::Bullet => AmmoShape::Ball {
                radius: 2.5,
                density: 0.113,
                restitution: 0.05,
                friction: 0.3,
                color: Color::srgb(0.95, 0.8, 0.3),
            },
            Ammo::ClaySlug => AmmoShape::Cluster {
                material: MaterialKind::Clay,
                radius_cells: 2,
                cell: 6.0,
            },
            Ammo::GlassMarble => AmmoShape::Cluster {
                material: MaterialKind::Glass,
                radius_cells: 2,
                cell: 6.0,
            },
        }
    }
}

#[derive(Resource)]
pub struct Gun {
    pub pos: Vec2,
    pub dir: Vec2,
    pub ammo: Ammo,
    pub speed: f32,
    /// Shots per second while the fire button is held; 0 means one shot per click.
    pub auto_rate: f32,
    pub cooldown: f32,
}

impl Default for Gun {
    fn default() -> Self {
        Self {
            pos: Vec2::new(-700.0, -330.0),
            dir: Vec2::X,
            ammo: Ammo::Cannonball,
            speed: Ammo::Cannonball.default_speed(),
            auto_rate: 0.0,
            cooldown: 0.0,
        }
    }
}

impl Gun {
    /// Advances the trigger by `dt` and says whether to fire now. Without auto-fire, each click
    /// fires once. With auto-fire, the gun fires as soon as the button goes down and then once
    /// per interval while it is held. The cooldown is clamped to one interval, so raising the
    /// rate takes effect at once and a slow frame catches up by at most one shot.
    pub fn trigger(&mut self, held: bool, just_pressed: bool, dt: f32) -> bool {
        if self.auto_rate <= 0.0 {
            self.cooldown = 0.0;
            return just_pressed;
        }
        let interval = 1.0 / self.auto_rate;
        if !held {
            self.cooldown = 0.0;
            return false;
        }
        self.cooldown = (self.cooldown - dt).clamp(-interval, interval);
        if self.cooldown > 0.0 {
            return false;
        }
        self.cooldown += interval;
        true
    }
}

/// Something the gun fired (a ball, or the root of a cluster).
#[derive(Component)]
pub struct Projectile;

/// A round body; visuals attach a circle mesh to it.
#[derive(Component)]
pub struct Round {
    pub radius: f32,
    pub color: Color,
}

/// Seconds until a projectile (or a cluster projectile's group root) is removed.
#[derive(Component)]
pub struct Lifetime(pub f32);

pub const MUZZLE: f32 = 36.0;

/// The gun is player one's.
fn gunner() -> HitTag {
    HitTag {
        owner: Some(Player::One),
        last_hit: None,
    }
}
const PROJECTILE_LIFETIME: f32 = 20.0;

pub fn fire(commands: &mut Commands, materials: &Materials, anchor: Entity, gun: &Gun) {
    let dir = gun.dir.normalize_or(Vec2::X);
    let pos = gun.pos + dir * MUZZLE;
    let velocity = dir * gun.speed;
    match gun.ammo.shape() {
        AmmoShape::Ball {
            radius,
            density,
            restitution,
            friction,
            color,
        } => {
            commands.spawn((
                Projectile,
                LevelEntity,
                gunner(),
                CollisionEventsEnabled,
                Round { radius, color },
                Lifetime(PROJECTILE_LIFETIME),
                RigidBody::Dynamic,
                Collider::circle(radius),
                ColliderDensity(density),
                Restitution::new(restitution),
                Friction::new(friction),
                LinearVelocity(velocity),
                SweptCcd::default(),
                Transform::from_translation(pos.extend(1.0)),
            ));
        }
        AmmoShape::Cluster {
            material,
            radius_cells,
            cell,
        } => {
            let n = radius_cells * 2 + 1;
            let spec = LatticeSpec {
                center: pos,
                angle: 0.0,
                cols: n,
                rows: n,
                cell,
                strain_length: cell,
                material,
                velocity,
                round: true,
                pins: vec![],
                ccd: true,
                can_sleep: true,
                bounce: None,
                friction: None,
            };
            let root = commands
                .spawn((
                    GroupRoot,
                    Projectile,
                    LevelEntity,
                    gunner(),
                    ReportHits,
                    Lifetime(PROJECTILE_LIFETIME),
                ))
                .id();
            spawn_lattice(
                commands,
                materials,
                anchor,
                Isometry2d::IDENTITY,
                &spec,
                root,
            );
        }
    }
}

pub fn tick_lifetimes(
    mut commands: Commands,
    time: Res<Time>,
    mut lifetimes: Query<(Entity, &mut Lifetime, Has<GroupRoot>)>,
    members: Query<(Entity, &Group)>,
) {
    for (entity, mut lifetime, is_group) in &mut lifetimes {
        lifetime.0 -= time.delta_secs();
        if lifetime.0 > 0.0 {
            continue;
        }
        if is_group {
            doom_group(&mut commands, entity, &members);
        } else {
            commands.entity(entity).insert(Doomed);
        }
    }
}

/// Fires at a fixed target with the low ballistic arc, falling back to a straight shot when the
/// target is out of range. Used by the probe.
pub fn aim_at(from: Vec2, to: Vec2, speed: f32, gravity: f32) -> Vec2 {
    let d = to - from;
    if gravity <= 0.0 || d.x.abs() < 1.0 {
        return d.normalize_or(Vec2::X);
    }
    let (v2, g) = (speed * speed, gravity);
    let disc = v2 * v2 - g * (g * d.x * d.x + 2.0 * d.y * v2);
    if disc < 0.0 {
        return d.normalize_or(Vec2::X);
    }
    let angle = ((v2 - disc.sqrt()) / (g * d.x.abs())).atan();
    Vec2::new(angle.cos() * d.x.signum(), angle.sin())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shots fired while holding (or, without auto-fire, clicking every frame) for `seconds`.
    fn shots(gun: &mut Gun, seconds: f32, fps: f32) -> usize {
        let frames = (seconds * fps) as usize;
        (0..frames)
            .filter(|&frame| gun.trigger(true, frame == 0, 1.0 / fps))
            .count()
    }

    #[test]
    fn single_shot_fires_once_per_click() {
        let mut gun = Gun::default();
        assert_eq!(shots(&mut gun, 1.0, 60.0), 1);
    }

    #[test]
    fn auto_fire_works_right_after_single_shots() {
        // The bug: a single shot used to leave a 1000 s cooldown behind.
        let mut gun = Gun::default();
        assert!(gun.trigger(true, true, 1.0 / 60.0));
        gun.auto_rate = 10.0;
        let fired = shots(&mut gun, 1.0, 60.0);
        assert!(
            (9..=11).contains(&fired),
            "fired {fired} shots in 1 s at 10/s"
        );
    }

    #[test]
    fn auto_fire_keeps_its_rate_and_does_not_burst_after_a_pause() {
        let mut gun = Gun {
            auto_rate: 20.0,
            ..default()
        };
        let fired = shots(&mut gun, 2.0, 144.0);
        assert!(
            (39..=41).contains(&fired),
            "fired {fired} shots in 2 s at 20/s"
        );
        // Released for a while: the next press fires once, not a backlog.
        for _ in 0..300 {
            gun.trigger(false, false, 1.0 / 60.0);
        }
        assert!(gun.trigger(true, true, 1.0 / 60.0));
        assert!(!gun.trigger(true, false, 1.0 / 60.0));
    }
}
