//! Impact effects: dust and sparks where bonds break, explosion flashes, and (optional) screen
//! shake and slow motion when a lot breaks at once.

use avian2d::prelude::Gravity;
use bevy::prelude::*;

use crate::fracture::Breaks;
use crate::play::{Explosions, Mode};
use crate::visuals::{View, WorldCamera};

pub struct EffectsPlugin;

impl Plugin for EffectsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Shake>()
            .init_resource::<SlowMotion>()
            .init_resource::<Flashes>()
            .add_systems(
                Update,
                (
                    react_to_breaks,
                    move_particles,
                    draw_flashes,
                    end_slow_motion,
                )
                    .chain(),
            )
            .add_systems(PostUpdate, shake_camera.before(TransformSystems::Propagate));
    }
}

/// A short-lived speck of dust, splinter or spark (purely visual, not simulated).
#[derive(Component)]
struct Particle {
    velocity: Vec2,
    life: f32,
    max_life: f32,
    alpha: f32,
}

/// Screen shake: grows with breaks and explosions, decays over time.
#[derive(Resource, Default)]
struct Shake {
    trauma: f32,
}

/// While set, time runs slow until `until` (real seconds), then `restore` is put back.
#[derive(Resource, Default)]
struct SlowMotion {
    until: f32,
    restore: Option<f32>,
}

/// Expanding rings where explosions went off: (position, radius, real start time).
#[derive(Resource, Default)]
struct Flashes(Vec<(Vec2, f32, f32)>);

const MAX_PARTICLES: usize = 1500;
/// Breaks in one frame that count as a big event.
const BIG_EVENT_BREAKS: usize = 40;
const SLOW_MOTION_SPEED: f32 = 0.35;
const SLOW_MOTION_SECONDS: f32 = 0.7;

/// A tiny xorshift generator; effects only need cheap, varied numbers.
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 >> 8) as f32 / (1 << 24) as f32
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next()
    }

    fn direction(&mut self) -> Vec2 {
        Vec2::from_angle(self.range(0.0, std::f32::consts::TAU))
    }
}

fn spawn_particle(
    commands: &mut Commands,
    pos: Vec2,
    velocity: Vec2,
    color: Color,
    size: f32,
    life: f32,
) {
    commands.spawn((
        Particle {
            velocity,
            life,
            max_life: life,
            alpha: color.alpha(),
        },
        Sprite::from_color(color, Vec2::splat(size)),
        Transform::from_translation(pos.extend(3.0)),
    ));
}

fn react_to_breaks(
    mut commands: Commands,
    breaks: Res<Breaks>,
    explosions: Res<Explosions>,
    particles: Query<(), With<Particle>>,
    view: Res<View>,
    mode: Res<State<Mode>>,
    real: Res<Time<Real>>,
    mut time: ResMut<Time<Virtual>>,
    mut shake: ResMut<Shake>,
    mut slow: ResMut<SlowMotion>,
    mut flashes: ResMut<Flashes>,
    mut last_seen: Local<f32>,
    mut seed: Local<u32>,
) {
    let mut rng = Rng(seed.wrapping_mul(747_796_405).wrapping_add(2_891_336_453) | 1);
    let now = time.elapsed_secs();
    let fresh: Vec<_> = breaks
        .0
        .iter()
        .filter(|b| b.time > *last_seen)
        .copied()
        .collect();
    if let Some(latest) = breaks.0.iter().map(|b| b.time).reduce(f32::max) {
        *last_seen = last_seen.max(latest);
    }
    *last_seen = last_seen.min(now);

    let mut budget = MAX_PARTICLES.saturating_sub(particles.iter().count());
    for b in &fresh {
        for _ in 0..3.min(budget) {
            let color = b.color.mix(&Color::WHITE, rng.range(0.0, 0.35));
            let velocity = rng.direction() * rng.range(40.0, 170.0) + Vec2::Y * 40.0;
            spawn_particle(
                &mut commands,
                b.pos,
                velocity,
                color,
                rng.range(1.5, 3.2),
                rng.range(0.3, 0.8),
            );
            budget -= 1;
        }
    }
    for e in &explosions.0 {
        flashes.0.push((e.pos, e.radius, real.elapsed_secs()));
        let speed = 420.0 * e.power.sqrt();
        for _ in 0..40.min(budget) {
            let heat = rng.next();
            let color = Color::srgb(1.0, 0.35 + 0.55 * heat, 0.1 + 0.3 * heat * heat);
            let velocity = rng.direction() * rng.range(0.2, 1.0) * speed;
            spawn_particle(
                &mut commands,
                e.pos,
                velocity,
                color,
                rng.range(2.0, 4.5),
                rng.range(0.25, 0.9),
            );
            budget -= 1;
        }
    }
    *seed = rng.0;

    if !view.impact_fx || *mode.get() != Mode::Play {
        return;
    }
    let blast: f32 = explosions.0.iter().map(|e| e.power).sum();
    shake.trauma = (shake.trauma + fresh.len() as f32 * 0.012 + blast * 0.45).min(1.0);
    let big = fresh.len() >= BIG_EVENT_BREAKS || blast >= 1.0;
    if big && (slow.restore.is_some() || time.relative_speed() > SLOW_MOTION_SPEED) {
        if slow.restore.is_none() {
            slow.restore = Some(time.relative_speed());
            time.set_relative_speed(SLOW_MOTION_SPEED);
        }
        slow.until = real.elapsed_secs() + SLOW_MOTION_SECONDS;
    }
}

fn end_slow_motion(
    mut slow: ResMut<SlowMotion>,
    real: Res<Time<Real>>,
    mut time: ResMut<Time<Virtual>>,
) {
    if real.elapsed_secs() >= slow.until
        && let Some(speed) = slow.restore.take()
    {
        time.set_relative_speed(speed);
    }
}

fn move_particles(
    mut commands: Commands,
    time: Res<Time<Virtual>>,
    gravity: Res<Gravity>,
    mut particles: Query<(Entity, &mut Particle, &mut Transform, &mut Sprite)>,
) {
    let dt = time.delta_secs();
    for (entity, mut p, mut transform, mut sprite) in &mut particles {
        p.life -= dt;
        if p.life <= 0.0 {
            commands.entity(entity).despawn();
            continue;
        }
        let drag = (1.0 - 2.5 * dt).max(0.0);
        p.velocity = p.velocity * drag + gravity.0 * 0.5 * dt;
        transform.translation += (p.velocity * dt).extend(0.0);
        let fade = (p.life / p.max_life).clamp(0.0, 1.0);
        sprite.color = sprite.color.with_alpha(p.alpha * fade);
    }
}

fn draw_flashes(mut gizmos: Gizmos, mut flashes: ResMut<Flashes>, real: Res<Time<Real>>) {
    const DURATION: f32 = 0.35;
    let now = real.elapsed_secs();
    flashes.0.retain(|(_, _, start)| now - start < DURATION);
    for (pos, radius, start) in &flashes.0 {
        let t = (now - start) / DURATION;
        let color = Color::srgba(1.0, 0.8, 0.4, 1.0 - t);
        gizmos.circle_2d(*pos, radius * (0.3 + 0.7 * t), color);
        gizmos.circle_2d(*pos, radius * 0.6 * t, color.with_alpha((1.0 - t) * 0.5));
    }
}

/// Offsets the world camera while playing. In Play mode the camera sits at the origin, so the
/// offset is the whole translation; the editor's camera is left alone.
fn shake_camera(
    mut shake: ResMut<Shake>,
    real: Res<Time<Real>>,
    mode: Res<State<Mode>>,
    mut cameras: Query<(&mut Transform, &Projection), With<WorldCamera>>,
    mut seed: Local<u32>,
) {
    if *mode.get() != Mode::Play {
        shake.trauma = 0.0;
        return;
    }
    let Ok((mut transform, projection)) = cameras.single_mut() else {
        return;
    };
    let scale = match projection {
        Projection::Orthographic(ortho) => ortho.scale,
        _ => 1.0,
    };
    let mut rng = Rng(seed.wrapping_mul(1_103_515_245).wrapping_add(12_345) | 1);
    let offset = if shake.trauma > 0.0 {
        rng.direction() * shake.trauma * shake.trauma * 14.0 * scale
    } else {
        Vec2::ZERO
    };
    *seed = rng.0;
    shake.trauma = (shake.trauma - 1.6 * real.delta_secs()).max(0.0);
    let target = offset.extend(transform.translation.z);
    if transform.translation != target {
        transform.translation = target;
    }
}
