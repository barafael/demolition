//! Headless tuning harness.
//!
//! - `--probe`: (1) every Lab structure built from each material settles under gravity; ideally
//!   nothing breaks. (2) Every ammo type is fired at every structure. (3) Pong runs unattended.
//! - `--diag [substeps]`: peak strain from gravity alone, with yielding and breaking disabled.
//! - `--pong`: just the unattended Pong run.
//! - `--rest`: resting speeds per structure (what keeps them from sleeping).
//! - `--bench`: cost of a physics step, by phase.
//!
//! `--resolution X` and `--substeps N` override every probed level's physics resolution and
//! substep count, for trying finer cells or cheaper steps across the whole suite.

use std::collections::HashMap;
use std::time::Duration;

use avian2d::collision::CollisionDiagnostics;
use avian2d::dynamics::solver::SolverDiagnostics;
use avian2d::prelude::*;
use bevy::input::InputPlugin;
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use bevy::time::TimeUpdateStrategy;

use crate::SimPlugin;
use crate::fracture::Stats;
use crate::gun::{Ammo, Gun, aim_at, fire};
use crate::lattice::{Bond, Cell, Group, WorldAnchor};
use crate::level::{self, Body, Level};
use crate::materials::{MaterialKind, Materials};
use crate::play::{Driven, ElementRoot, Mode, Score};

const HZ: f64 = 64.0;

/// `--resolution X` on the command line overrides every probed level's physics resolution.
fn resolution_override() -> Option<f32> {
    let args: Vec<String> = std::env::args().collect();
    let i = args.iter().position(|a| a == "--resolution")?;
    args.get(i + 1)?.parse().ok()
}

fn make_app(mut level: Level) -> App {
    if let Some(r) = resolution_override() {
        level.resolution = r;
    }
    let args: Vec<String> = std::env::args().collect();
    if let Some(n) = args
        .iter()
        .position(|a| a == "--substeps")
        .and_then(|i| args.get(i + 1)?.parse().ok())
    {
        level.substeps = n;
    }
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        TransformPlugin,
        StatesPlugin,
        InputPlugin,
        SimPlugin,
    ))
    .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
        1.0 / HZ,
    )))
    .insert_resource(level);
    app.finish();
    app.cleanup();
    app.world_mut()
        .resource_mut::<NextState<Mode>>()
        .set(Mode::Play);
    app.update();
    app
}

fn step(app: &mut App, seconds: f32) {
    for _ in 0..(seconds as f64 * HZ) as usize {
        app.update();
    }
}

/// The Lab preset with every lattice made of `material`.
fn lab(material: MaterialKind) -> Level {
    let mut level = level::preset_lab();
    for element in &mut level.elements {
        if let Body::Lattice { material: m, .. } = &mut element.body {
            *m = material;
        }
    }
    level
}

fn lattice_indices(level: &Level) -> Vec<usize> {
    (0..level.elements.len())
        .filter(|&i| matches!(level.elements[i].body, Body::Lattice { .. }))
        .collect()
}

/// Element index of every live element root.
fn root_indices(world: &mut World) -> HashMap<Entity, usize> {
    world
        .query::<(Entity, &ElementRoot)>()
        .iter(world)
        .map(|(e, r)| (e, r.index))
        .collect()
}

/// Intact (bonds, pins) per element index.
fn count_bonds(world: &mut World) -> HashMap<usize, (usize, usize)> {
    let roots = root_indices(world);
    let mut counts: HashMap<usize, (usize, usize)> = HashMap::new();
    for (bond, group) in world.query::<(&Bond, &Group)>().iter(world) {
        if let Some(&i) = roots.get(&group.0) {
            let c = counts.entry(i).or_default();
            if bond.material.is_some() {
                c.0 += 1;
            } else {
                c.1 += 1;
            }
        }
    }
    counts
}

fn cell_positions(world: &mut World) -> HashMap<Entity, Vec2> {
    world
        .query_filtered::<(Entity, &Position), With<Cell>>()
        .iter(world)
        .map(|(e, p)| (e, p.0))
        .collect()
}

fn max_displacement(world: &mut World, start: &HashMap<Entity, Vec2>) -> HashMap<usize, f32> {
    let roots = root_indices(world);
    let mut out: HashMap<usize, f32> = HashMap::new();
    for (e, p, group) in world.query::<(Entity, &Position, &Group)>().iter(world) {
        let (Some(&i), Some(s)) = (roots.get(&group.0), start.get(&e)) else {
            continue;
        };
        let d = out.entry(i).or_default();
        *d = d.max(p.0.distance(*s));
    }
    out
}

/// Total plastic damage and peak cell speed of one element.
fn plastic_and_speed(world: &mut World, index: usize) -> (f32, f32) {
    let roots = root_indices(world);
    let mut plastic = 0.0f32;
    for (bond, group) in world.query::<(&Bond, &Group)>().iter(world) {
        if roots.get(&group.0) == Some(&index) {
            plastic += bond.damage;
        }
    }
    let mut speed = 0.0f32;
    for (v, group) in world
        .query_filtered::<(&LinearVelocity, &Group), With<Cell>>()
        .iter(world)
    {
        if roots.get(&group.0) == Some(&index) {
            speed = speed.max(v.length());
        }
    }
    (plastic, speed)
}

fn lost(
    before: &HashMap<usize, (usize, usize)>,
    after: &HashMap<usize, (usize, usize)>,
    i: usize,
) -> (usize, usize) {
    let b = before.get(&i).copied().unwrap_or_default();
    let a = after.get(&i).copied().unwrap_or_default();
    (b.0.saturating_sub(a.0), b.1.saturating_sub(a.1))
}

/// Where the probe shoots a structure from: above for long flat ones, from the left otherwise.
fn test_shot(level: &Level, index: usize) -> (Vec2, Vec2) {
    let element = &level.elements[index];
    let size = element.size();
    let target = element.pos;
    if size.x > size.y * 3.0 {
        (target + Vec2::new(0.0, 280.0), target)
    } else {
        (target + Vec2::new(-320.0, 0.0), target)
    }
}

pub fn run() {
    let template = level::preset_lab();
    let lattices = lattice_indices(&template);
    let name = |i: usize| template.elements[i].name.clone();

    println!("== Settle under gravity, 4 s: broken / pins lost / sag ==");
    println!(
        "{:<9}{}",
        "",
        lattices
            .iter()
            .map(|&i| format!("{:>18}", name(i)))
            .collect::<String>()
    );
    for material in MaterialKind::ALL {
        let mut app = make_app(lab(material));
        step(&mut app, 2.0 / HZ as f32);
        let before = count_bonds(app.world_mut());
        let start = cell_positions(app.world_mut());
        step(&mut app, 4.0);
        let after = count_bonds(app.world_mut());
        let sag = max_displacement(app.world_mut(), &start);
        let row: String = lattices
            .iter()
            .map(|&i| {
                let (b, p) = lost(&before, &after, i);
                format!(
                    "{:>18}",
                    format!("{b}/{p}/{:.1}px", sag.get(&i).unwrap_or(&0.0))
                )
            })
            .collect();
        println!(
            "{:<9}{row}   plastic {:.2}",
            material.name(),
            app.world().resource::<Stats>().plastic
        );
    }

    println!();
    println!("== Shots, 2 s: broken bonds (pins lost) plastic-damage peak-cell-speed ==");
    println!(
        "{:<13}{}",
        "",
        lattices
            .iter()
            .map(|&i| format!("{:>24}", name(i)))
            .collect::<String>()
    );
    for material in MaterialKind::ALL {
        println!("-- {}", material.name());
        for ammo in Ammo::ALL {
            let mut row = String::new();
            for &index in &lattices {
                // Only this structure (plus all walls), so shots can't interfere.
                let mut level = lab(material);
                let (from, to) = test_shot(&level, index);
                let keep: Vec<bool> = (0..level.elements.len())
                    .map(|i| i == index || matches!(level.elements[i].body, Body::Wall { .. }))
                    .collect();
                let local = keep[..index].iter().filter(|&&k| k).count();
                let mut keep_iter = keep.iter();
                level.elements.retain(|_| *keep_iter.next().unwrap());

                let mut app = make_app(level);
                step(&mut app, 0.5);
                let before = count_bonds(app.world_mut());
                let world = app.world_mut();
                let materials = world.resource::<Materials>().clone();
                let anchor = world.resource::<WorldAnchor>().0;
                let g = -world.resource::<Gravity>().0.y;
                let gun = Gun {
                    pos: from,
                    dir: aim_at(from, to, ammo.default_speed(), g),
                    ammo,
                    speed: ammo.default_speed(),
                    ..default()
                };
                fire(&mut world.commands(), &materials, anchor, &gun);
                world.flush();
                let mut peak = (0.0f32, 0.0f32);
                for _ in 0..(2.0 * HZ) as usize {
                    app.update();
                    let (plastic, speed) = plastic_and_speed(app.world_mut(), local);
                    peak = (plastic, peak.1.max(speed));
                }
                let after = count_bonds(app.world_mut());
                let (b, p) = lost(&before, &after, local);
                row += &format!("{:>24}", format!("{b} ({p}) {:.1} {:.0}", peak.0, peak.1));
            }
            println!("{:<13}{row}", ammo.name());
        }
    }

    println!();
    pong();
}

/// Runs the Pong preset with nobody at the controls and reports what happened.
pub fn pong() {
    println!("== Pong preset, unattended ==");
    pong_run(level::preset_pong());
    gun_credit();
}

/// Shots from the gun belong to P1, so destroying a last-hitter element must score for P1.
fn gun_credit() {
    let level = level::preset_lab();
    let pillar = level.elements.iter().find(|e| e.name == "Pillar").unwrap();
    let (target, points) = (pillar.pos, pillar.points);
    let mut app = make_app(level);
    step(&mut app, 0.5);
    for _ in 0..4 {
        let world = app.world_mut();
        let materials = world.resource::<Materials>().clone();
        let anchor = world.resource::<WorldAnchor>().0;
        let from = target + Vec2::new(-320.0, 60.0);
        let gun = Gun {
            pos: from,
            dir: aim_at(from, target + Vec2::Y * 60.0, 1400.0, 900.0),
            ammo: Ammo::Cannonball,
            speed: 1400.0,
            ..default()
        };
        fire(&mut world.commands(), &materials, anchor, &gun);
        world.flush();
        step(&mut app, 1.0);
    }
    let score = app.world().resource::<Score>().points;
    let world = app.world_mut();
    let pillar = world
        .query::<(&ElementRoot, &crate::play::HitTag, &Name)>()
        .iter(world)
        .find(|(_, _, name)| name.as_str() == "Pillar")
        .map(|(root, tag, _)| (tag.last_hit, root.destroyed));
    println!("== Gun credit: 4 cannonballs at the Lab pillar ({points} pts, last hitter) ==");
    println!(
        "   pillar (last hit by, destroyed): {pillar:?}, score P1 {} : {} P2",
        score[0], score[1]
    );
}

fn pong_run(level: Level) {
    let mut app = make_app(level.clone());
    let seconds = 60.0;
    let mut balls_lost = 0;
    let mut destroyed_seen = HashMap::new();
    for _ in 0..(seconds * HZ) as usize {
        app.update();
        let world = app.world_mut();
        for (entity, root) in world.query::<(Entity, &ElementRoot)>().iter(world) {
            if root.destroyed
                && destroyed_seen.insert(entity, root.index).is_none()
                && level.elements[root.index].name == "Ball"
            {
                balls_lost += 1;
            }
        }
    }
    let mut names: Vec<String> = destroyed_seen
        .values()
        .map(|&i| level.elements[i].name.clone())
        .filter(|n| n != "Ball")
        .collect();
    names.sort();
    let score = app.world().resource::<Score>().points;
    let stats = app.world().resource::<Stats>();
    println!(
        "   {seconds} s: score {}:{}, {} bonds broken, {balls_lost} balls lost, destroyed: {}",
        score[0],
        score[1],
        stats.broken,
        if names.is_empty() {
            "none".into()
        } else {
            names.join(", ")
        }
    );
}

/// `--tnt`: a cannonball into a TNT charge next to a wooden pillar. Reports how many TNT cells
/// went off (chain reaction) and what the blast did to the pillar.
pub fn tnt() {
    let mut level = level::preset_empty();
    level.elements.push(crate::level::Element {
        name: "Charge".into(),
        pos: Vec2::new(0.0, -370.0),
        body: Body::Lattice {
            material: MaterialKind::Tnt,
            cols: 6,
            rows: 6,
            cell: 10.0,
            round: false,
        },
        pins: crate::level::Pins {
            bottom: true,
            ..default()
        },
        ..default()
    });
    level.elements.push(crate::level::Element {
        name: "Pillar".into(),
        pos: Vec2::new(70.0, -270.0),
        body: Body::Lattice {
            material: MaterialKind::Wood,
            cols: 3,
            rows: 26,
            cell: 10.0,
            round: false,
        },
        pins: crate::level::Pins {
            bottom: true,
            ..default()
        },
        ..default()
    });
    let mut app = make_app(level);
    step(&mut app, 0.5);
    let before = count_bonds(app.world_mut());
    let tnt_cells = |world: &mut World| {
        world
            .query::<&Cell>()
            .iter(world)
            .filter(|c| c.material == MaterialKind::Tnt)
            .count()
    };
    let charge = tnt_cells(app.world_mut());
    let world = app.world_mut();
    let materials = world.resource::<Materials>().clone();
    let anchor = world.resource::<WorldAnchor>().0;
    let from = Vec2::new(-400.0, -360.0);
    let gun = Gun {
        pos: from,
        dir: aim_at(from, Vec2::new(-25.0, -370.0), 1400.0, 900.0),
        ammo: Ammo::Cannonball,
        speed: 1400.0,
        ..default()
    };
    fire(&mut world.commands(), &materials, anchor, &gun);
    world.flush();
    let mut blasts = 0;
    for _ in 0..(3.0 * HZ) as usize {
        app.update();
        blasts += app.world().resource::<crate::play::Explosions>().0.len();
    }
    let after = count_bonds(app.world_mut());
    let left = tnt_cells(app.world_mut());
    let pillar = lost(&before, &after, 2);
    println!("== TNT: 6x6 charge hit by a cannonball, 3 s ==");
    println!(
        "   {blasts} explosions, {} of {charge} TNT cells gone, pillar lost {} bonds and {} pins",
        charge - left,
        pillar.0,
        pillar.1
    );
}

/// Peak geometric strain and bend angle per element, for bonds and pins separately.
fn peak_strains(world: &mut World, out: &mut HashMap<usize, [(f32, f32); 2]>) {
    let roots = root_indices(world);
    let bodies: HashMap<Entity, (Vec2, Rotation)> = world
        .query::<(Entity, &Position, &Rotation)>()
        .iter(world)
        .map(|(e, p, r)| (e, (p.0, *r)))
        .collect();
    for (joint, bond, group) in world.query::<(&FixedJoint, &Bond, &Group)>().iter(world) {
        let Some(&i) = roots.get(&group.0) else {
            continue;
        };
        let (Some(&(p1, r1)), Some(&(p2, r2))) =
            (bodies.get(&joint.body1), bodies.get(&joint.body2))
        else {
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
        let strain = ((p2 + r2 * a2) - (p1 + r1 * a1)).length() / bond.cell_size;
        let angle = (r1 * b1).angle_between(r2 * b2).abs();
        let slot = &mut out.entry(i).or_default()[bond.material.is_none() as usize];
        slot.0 = slot.0.max(strain);
        slot.1 = slot.1.max(angle);
    }
}

pub fn diag() {
    let substeps = std::env::args()
        .skip_while(|a| a != "--diag")
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(32);
    let template = level::preset_lab();
    let lattices = lattice_indices(&template);
    println!("== substeps {substeps}: peak strain / angle after 1.5 s (bonds | pins) ==");
    println!(
        "{:<9}{}",
        "",
        lattices
            .iter()
            .map(|&i| format!("{:>34}", template.elements[i].name))
            .collect::<String>()
    );
    for material in MaterialKind::ALL {
        let mut level = lab(material);
        level.substeps = substeps;
        let m = &mut level.materials;
        for s in m
            .table
            .iter_mut()
            .map(|p| &mut p.strength)
            .chain([&mut m.pins])
        {
            s.yield_strain = 1e9;
            s.yield_angle = 1e9;
            s.break_strain = 1e9;
            s.break_angle = 1e9;
        }
        let mut app = make_app(level);
        let mut out = HashMap::new();
        for i in 0..(4.0 * HZ) as usize {
            app.update();
            if i as f64 >= 1.5 * HZ {
                peak_strains(app.world_mut(), &mut out);
            }
        }
        let row: String = lattices
            .iter()
            .map(|i| {
                let [b, p] = out.get(i).copied().unwrap_or_default();
                format!(
                    "{:>34}",
                    format!("{:.3}/{:.3} | {:.3}/{:.3}", b.0, b.1, p.0, p.1)
                )
            })
            .collect();
        println!("{:<9}{row}", material.name());
    }
}

/// CPU time (user + system) this process has used, in seconds. Unlike wall time it barely
/// changes when other programs compete for the CPU, so benchmark comparisons stay meaningful.
/// Linux only; `None` elsewhere.
fn process_cpu_seconds() -> Option<f64> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    // Fields after the parenthesised command name; utime and stime are the 12th and 13th.
    let rest = stat.rsplit_once(')')?.1;
    let fields: Vec<&str> = rest.split_whitespace().collect();
    let ticks: f64 = fields.get(11)?.parse::<f64>().ok()? + fields.get(12)?.parse::<f64>().ok()?;
    Some(ticks / 100.0)
}

/// Physics step cost for each preset, after settling, and for the Lab after a barrage of
/// cannonballs has filled it with debris. Also reports the number of contact pairs the
/// narrow phase tracks, since that is what debris piles inflate.
/// `--rest`: how still every preset is after settling for 3 s. Structures that keep moving
/// never sleep, so this shows where simulation time goes on nothing visible.
pub fn rest_speeds() {
    let cases = bench_cases()
        .into_iter()
        // The debris case only differs by the barrage, which this probe doesn't fire.
        .filter(|(_, _, debris)| !*debris);
    for (name, level, _) in cases {
        println!("== {name} ==");
        rest_speeds_of(level);
    }
}

fn rest_speeds_of(level: Level) {
    let mut app = make_app(level.clone());
    step(&mut app, 3.0);
    let broken = app.world().resource::<Stats>().broken;
    let world = app.world_mut();
    let roots = root_indices(world);
    let mut out: HashMap<usize, Vec<(f32, f32, Vec2, Option<u8>)>> = HashMap::new();
    for (v, w, p, g, c) in world
        .query_filtered::<(
            &LinearVelocity,
            &AngularVelocity,
            &Position,
            &Group,
            Option<&Cell>,
        ), Without<Driven>>()
        .iter(world)
    {
        if let Some(&i) = roots.get(&g.0) {
            out.entry(i)
                .or_default()
                .push((v.length(), w.0.abs(), p.0, c.map(|c| c.bonds)));
        }
    }
    let mut rows: Vec<(String, String)> = out
        .into_iter()
        .map(|(i, mut cells)| {
            cells.sort_by(|a, b| b.0.total_cmp(&a.0));
            let n = cells.len();
            let mean = cells.iter().map(|c| c.0).sum::<f32>() / n as f32;
            let fast = cells.iter().filter(|c| c.0 > 1.0).count();
            let top: Vec<String> = cells
                .iter()
                .take(3)
                .map(|c| {
                    format!(
                        "{:.0}px/s@({:.0},{:.0}) bonds {}",
                        c.0,
                        c.2.x,
                        c.2.y,
                        c.3.map(|b| b.to_string()).unwrap_or_else(|| "-".into())
                    )
                })
                .collect();
            (
                level.elements[i].name.clone(),
                format!(
                    "{n} bodies, mean {mean:.2} px/s, {fast} above 1 px/s; fastest: {}",
                    top.join(", ")
                ),
            )
        })
        .collect();
    rows.sort();
    for (name, report) in rows {
        println!("{:<14} {report}", name);
    }
    println!("{:<14} bonds broken while settling: {broken}", "total");
}

/// Every preset, plus the Lab again after a debris barrage: the benchmark and rest probe
/// run over all of them.
fn bench_cases() -> Vec<(&'static str, Level, bool)> {
    vec![
        ("lab", level::preset_lab(), false),
        ("lab + debris", level::preset_lab(), true),
        ("pong", level::preset_pong(), false),
        ("tower", level::preset_tower(), false),
        ("wreck", level::preset_wreck(), false),
        ("domino", level::preset_domino(), false),
        ("pyramid", level::preset_pyramid(), false),
    ]
}

pub fn bench() {
    let barrage = |app: &mut App| {
        let world = app.world_mut();
        let materials = world.resource::<Materials>().clone();
        let anchor = world.resource::<WorldAnchor>().0;
        for k in 0..12 {
            let from = Vec2::new(-700.0, -330.0 + 40.0 * (k % 4) as f32);
            let to = Vec2::new(
                -200.0 + 120.0 * (k % 6) as f32,
                -150.0 + 60.0 * (k / 6) as f32,
            );
            let gun = Gun {
                pos: from,
                dir: aim_at(from, to, 1400.0, 900.0),
                ammo: Ammo::Cannonball,
                speed: 1400.0,
                ..default()
            };
            fire(&mut world.commands(), &materials, anchor, &gun);
        }
        world.flush();
    };
    let cases = bench_cases();
    for (name, level, debris) in cases {
        let mut app = make_app(level);
        step(&mut app, 0.5);
        if debris {
            barrage(&mut app);
            step(&mut app, 0.5);
            barrage(&mut app);
        }
        step(&mut app, 2.5);
        let phases = |world: &World| {
            let c = world.get_resource::<CollisionDiagnostics>();
            let s = world.get_resource::<SolverDiagnostics>();
            [
                c.map(|c| c.broad_phase),
                c.map(|c| c.narrow_phase),
                s.map(|s| {
                    s.prepare_constraints
                        + s.update_velocity_increments
                        + s.integrate_velocities
                        + s.warm_start
                        + s.solve_constraints
                        + s.integrate_positions
                        + s.relax_velocities
                        + s.apply_restitution
                        + s.finalize
                        + s.store_impulses
                        + s.swept_ccd
                }),
            ]
            .map(|d| d.unwrap_or_default().as_secs_f64())
        };
        let start = std::time::Instant::now();
        let cpu_start = process_cpu_seconds();
        let n = 256;
        // The diagnostics are reset every frame, so sum them frame by frame.
        let mut phase_sum = [0.0f64; 3];
        for _ in 0..n {
            app.update();
            for (sum, d) in phase_sum.iter_mut().zip(phases(app.world())) {
                *sum += d;
            }
        }
        let ms = start.elapsed().as_secs_f64() * 1000.0 / n as f64;
        let cpu_ms = process_cpu_seconds()
            .zip(cpu_start)
            .map_or(f64::NAN, |(end, start)| (end - start) * 1000.0 / n as f64);
        let [broad, narrow, solver] = phase_sum.map(|d| d * 1000.0 / n as f64);
        let world = app.world_mut();
        let bodies = world.query::<&RigidBody>().iter(world).count();
        let sleeping = world
            .query_filtered::<(), With<Sleeping>>()
            .iter(world)
            .count();
        let graph = world.resource::<ContactGraph>();
        let pairs = graph.active_pairs().len();
        let touching = graph.iter_active_touching().count();
        // Same frames with physics paused: what our own systems and Bevy's schedule cost.
        app.world_mut().resource_mut::<Time<Physics>>().pause();
        let start = std::time::Instant::now();
        for _ in 0..n {
            app.update();
        }
        let idle_ms = start.elapsed().as_secs_f64() * 1000.0 / n as f64;
        app.world_mut().resource_mut::<Time<Physics>>().unpause();
        println!("{name:<14} frame without physics: {idle_ms:.2} ms");
        println!(
            "{name:<14} {ms:5.2} ms/step wall {cpu_ms:5.2} CPU | broad {broad:4.2} narrow {narrow:4.2} \
             contact solver {solver:4.2} rest {:4.2} | {bodies} bodies ({sleeping} asleep), \
             {pairs} pairs ({touching} touching)",
            ms - broad - narrow - solver
        );
    }
}
