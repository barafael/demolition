use avian2d::prelude::*;
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};

use crate::editor::{Editor, History, HistoryStep, new_element};
use crate::fracture::Stats;
use crate::gun::{Ammo, Gun};
use crate::level::{self, Credit, Highscores, Level, Player};
use crate::materials::{MaterialKind, Materials, Strength};
use crate::play::{ClearDebris, Mode, Restart, Score};
use crate::visuals::View;

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LevelFiles>().add_systems(
            EguiPrimaryContextPass,
            (panel, score_hud.run_if(in_state(Mode::Play))),
        );
    }
}

/// Saved level names, refreshed on demand, the last file operation's outcome, and the text box
/// for pasting a shared level.
#[derive(Resource)]
struct LevelFiles {
    names: Vec<String>,
    status: String,
    import: String,
}

impl Default for LevelFiles {
    fn default() -> Self {
        Self {
            names: level::list(),
            status: String::new(),
            import: String::new(),
        }
    }
}

fn strength_sliders(ui: &mut egui::Ui, s: &mut Strength) {
    ui.add(
        egui::Slider::new(&mut s.point_compliance, 1e-9..=1e-2)
            .logarithmic(true)
            .text("point compliance"),
    );
    ui.add(
        egui::Slider::new(&mut s.angle_compliance, 1e-9..=1e-2)
            .logarithmic(true)
            .text("angle compliance"),
    );
    ui.add(egui::Slider::new(&mut s.yield_strain, 0.0..=2.0).text("yield strain"));
    ui.add(egui::Slider::new(&mut s.yield_angle, 0.0..=3.0).text("yield angle"));
    ui.add(egui::Slider::new(&mut s.ductility, 0.0..=30.0).text("ductility"));
    ui.add(egui::Slider::new(&mut s.break_strain, 0.0..=3.0).text("break strain"));
    ui.add(egui::Slider::new(&mut s.break_angle, 0.0..=3.2).text("break angle"));
}

fn materials_section(ui: &mut egui::Ui, materials: &mut Materials) {
    egui::CollapsingHeader::new("Materials").show(ui, |ui| {
        ui.small(
            "Saved with the level. Edits apply live. Strains are in cell sizes, angles in radians.",
        );
        for (kind, params) in MaterialKind::ALL.into_iter().zip(&mut materials.table) {
            egui::CollapsingHeader::new(kind.name()).show(ui, |ui| {
                ui.add(
                    egui::Slider::new(&mut params.density, 0.001..=0.2)
                        .logarithmic(true)
                        .text("density"),
                );
                ui.add(egui::Slider::new(&mut params.friction, 0.0..=1.5).text("friction"));
                ui.add(egui::Slider::new(&mut params.restitution, 0.0..=1.0).text("restitution"));
                strength_sliders(ui, &mut params.strength);
                ui.small("density/friction/restitution apply on (re)spawn");
            });
        }
        egui::CollapsingHeader::new("Pins (bolts)").show(ui, |ui| {
            strength_sliders(ui, &mut materials.pins);
        });
        if ui.button("Reset materials").clicked() {
            *materials = Materials::default();
        }
    });
}

fn panel(
    mut contexts: EguiContexts,
    mode: Res<State<Mode>>,
    mut next_mode: ResMut<NextState<Mode>>,
    mut level: ResMut<Level>,
    mut editor: ResMut<Editor>,
    mut files: ResMut<LevelFiles>,
    mut history: ResMut<History>,
    mut gun: ResMut<Gun>,
    mut view: ResMut<View>,
    mut clear: ResMut<ClearDebris>,
    mut restart: ResMut<Restart>,
    mut time: ResMut<Time<Virtual>>,
    real: Res<Time<Real>>,
    stats: Res<Stats>,
    bodies: Query<(), With<RigidBody>>,
    cameras: Query<&Transform, With<Camera2d>>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    let playing = *mode.get() == Mode::Play;
    // Work on copies so resources are only marked changed when something actually changed.
    let mut edited_level = level.bypass_change_detection().clone();

    egui::Window::new("Demolition lab")
        .default_pos([10.0, 10.0])
        .default_width(320.0)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .max_height(840.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let label = if playing {
                            "■ Edit (Tab)"
                        } else {
                            "▶ Play (Tab)"
                        };
                        if ui.button(label).clicked() {
                            next_mode.set(if playing { Mode::Edit } else { Mode::Play });
                        }
                        ui.label(format!(
                            "{:.0} fps · {} bodies",
                            1.0 / real.delta_secs().max(1e-6),
                            bodies.iter().count()
                        ));
                    });

                    if playing {
                        ui.label(format!("{} bonds · {} broken", stats.bonds, stats.broken));
                        ui.small(
                            "R restart · C clear debris · B stress overlay · T trajectory\n\
                         M slow-mo · Space pause · Tab back to editor",
                        );
                        ui.horizontal(|ui| {
                            if ui.button("Restart (R)").clicked() {
                                restart.0 = true;
                            }
                            if ui.button("Clear debris (C)").clicked() {
                                clear.0 = true;
                            }
                        });
                        if edited_level.gun {
                            ui.separator();
                            ui.heading("Gun");
                            ui.small("LMB fire · RMB move gun · wheel speed · 1-6 ammo");
                            for (k, ammo) in Ammo::ALL.into_iter().enumerate() {
                                if ui
                                    .selectable_label(
                                        gun.ammo == ammo,
                                        format!("{} {}", k + 1, ammo.name()),
                                    )
                                    .clicked()
                                {
                                    gun.ammo = ammo;
                                    gun.speed = ammo.default_speed();
                                }
                            }
                            ui.add(
                                egui::Slider::new(&mut gun.speed, 50.0..=6000.0)
                                    .logarithmic(true)
                                    .text("muzzle speed px/s"),
                            );
                            ui.add(
                                egui::Slider::new(&mut gun.auto_rate, 0.0..=30.0)
                                    .text("auto-fire /s (0 = off)"),
                            );
                        }
                        ui.separator();
                        let mut speed = time.relative_speed();
                        if ui
                            .add(
                                egui::Slider::new(&mut speed, 0.02..=1.0)
                                    .logarithmic(true)
                                    .text("time scale"),
                            )
                            .changed()
                        {
                            time.set_relative_speed(speed);
                        }
                        let mut paused = time.is_paused();
                        if ui.checkbox(&mut paused, "paused").changed() {
                            if paused {
                                time.pause();
                            } else {
                                time.unpause();
                            }
                        }
                        ui.checkbox(&mut view.stress_overlay, "stress overlay");
                        ui.checkbox(&mut view.trajectory, "trajectory preview");
                    } else {
                        ui.small(
                            "LMB select/drag · RMB/MMB pan · wheel zoom\n\
                         Q/E rotate (Shift fine) · arrows nudge · Ctrl+D duplicate · Del delete\n\
                         Ctrl+Z undo · Ctrl+Shift+Z / Ctrl+Y redo",
                        );
                        ui.horizontal(|ui| {
                            if ui
                                .add_enabled(history.can_undo(), egui::Button::new("Undo"))
                                .clicked()
                            {
                                history.request = Some(HistoryStep::Undo);
                            }
                            if ui
                                .add_enabled(history.can_redo(), egui::Button::new("Redo"))
                                .clicked()
                            {
                                history.request = Some(HistoryStep::Redo);
                            }
                        });

                        ui.separator();
                        ui.heading("Level");
                        ui.horizontal(|ui| {
                            ui.label("name");
                            ui.text_edit_singleline(&mut edited_level.name);
                        });
                        ui.horizontal(|ui| {
                            if ui.button("Save").clicked() {
                                files.status = match level::save(&edited_level) {
                                    Ok(place) => format!("saved to {place}"),
                                    Err(e) => format!("save failed: {e}"),
                                };
                                files.names = level::list();
                            }
                            let mut chosen = None;
                            let combo = egui::ComboBox::from_id_salt("load")
                                .selected_text("Load…")
                                .show_ui(ui, |ui| {
                                    for name in &files.names {
                                        if ui.selectable_label(false, name).clicked() {
                                            chosen = Some(name.clone());
                                        }
                                    }
                                });
                            if combo.response.clicked() {
                                files.names = level::list();
                            }
                            if let Some(name) = chosen {
                                match level::load(&name) {
                                    Ok(loaded) => {
                                        edited_level = loaded;
                                        editor.selected = None;
                                        files.status = format!("loaded {name}");
                                    }
                                    Err(e) => files.status = format!("load failed: {e}"),
                                }
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("presets");
                            for (name, preset) in [
                                ("Lab", level::preset_lab as fn() -> Level),
                                ("Pong", level::preset_pong),
                                ("Empty", level::preset_empty),
                            ] {
                                if ui.button(name).clicked() {
                                    edited_level = preset();
                                    editor.selected = None;
                                    files.status = format!("{name} preset (unsaved)");
                                }
                            }
                        });
                        egui::CollapsingHeader::new("Share").show(ui, |ui| {
                            if ui.button("Copy level as text").clicked() {
                                match level::to_ron(&edited_level) {
                                    Ok(text) => {
                                        ui.ctx().copy_text(text);
                                        files.status = "level copied to clipboard".into();
                                    }
                                    Err(e) => files.status = format!("copy failed: {e}"),
                                }
                            }
                            ui.add(
                                egui::TextEdit::multiline(&mut files.import)
                                    .hint_text("paste a level here")
                                    .desired_rows(3),
                            );
                            if ui.button("Import pasted level").clicked() {
                                match level::from_ron(&files.import) {
                                    Ok(imported) => {
                                        files.status = format!("imported {}", imported.name);
                                        edited_level = imported;
                                        editor.selected = None;
                                        files.import.clear();
                                    }
                                    Err(e) => files.status = format!("import failed: {e}"),
                                }
                            }
                        });
                        if !files.status.is_empty() {
                            ui.small(&files.status);
                        }

                        ui.separator();
                        ui.heading("Add");
                        let center = cameras
                            .single()
                            .map(|t| t.translation.truncate())
                            .unwrap_or_default();
                        ui.horizontal(|ui| {
                            for kind in ["Lattice", "Ball", "Wall"] {
                                if ui.button(format!("+ {kind}")).clicked() {
                                    edited_level
                                        .elements
                                        .push(new_element(kind, center.round()));
                                    editor.selected = Some(edited_level.elements.len() - 1);
                                }
                            }
                        });

                        ui.separator();
                        egui::CollapsingHeader::new("World settings").show(ui, |ui| {
                            ui.add(
                                egui::Slider::new(&mut edited_level.gravity, -3000.0..=3000.0)
                                    .text("gravity"),
                            );
                            ui.add(
                                egui::Slider::new(&mut edited_level.substeps, 1..=60)
                                    .text("substeps"),
                            );
                            ui.horizontal(|ui| {
                                ui.label("view");
                                ui.add(
                                    egui::DragValue::new(&mut edited_level.view.x)
                                        .range(100.0..=20000.0),
                                );
                                ui.add(
                                    egui::DragValue::new(&mut edited_level.view.y)
                                        .range(100.0..=20000.0),
                                );
                            });
                            ui.horizontal(|ui| {
                                ui.label("bounds ±");
                                ui.add(
                                    egui::DragValue::new(&mut edited_level.bounds.x)
                                        .range(100.0..=20000.0),
                                );
                                ui.add(
                                    egui::DragValue::new(&mut edited_level.bounds.y)
                                        .range(100.0..=20000.0),
                                );
                            });
                            ui.checkbox(&mut edited_level.gun, "gun (drag it in the editor)");
                            ui.add(
                                egui::Slider::new(&mut editor.snap, 0.0..=50.0).text("drag snap"),
                            );
                        });

                        egui::CollapsingHeader::new(format!(
                            "Elements ({})",
                            edited_level.elements.len()
                        ))
                        .default_open(true)
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .id_salt("elements")
                                .max_height(260.0)
                                .show(ui, |ui| {
                                    for (i, element) in edited_level.elements.iter().enumerate() {
                                        let label = format!(
                                            "{} ({})",
                                            if element.name.is_empty() {
                                                "unnamed"
                                            } else {
                                                &element.name
                                            },
                                            element.body.kind_name()
                                        );
                                        if ui
                                            .selectable_label(editor.selected == Some(i), label)
                                            .clicked()
                                        {
                                            editor.selected = Some(i);
                                        }
                                    }
                                });
                        });
                    }

                    ui.separator();
                    materials_section(ui, &mut edited_level.materials);
                });
        });

    if edited_level != *level {
        *level = edited_level;
    }
    Ok(())
}

fn score_hud(
    mut contexts: EguiContexts,
    level: Res<Level>,
    score: Res<Score>,
    highscores: Res<Highscores>,
) -> Result {
    if level.elements.iter().all(|e| e.points == 0) {
        return Ok(());
    }
    let two_players = level
        .elements
        .iter()
        .any(|e| e.owner == Some(Player::Two) || e.credit == Credit::Player(Player::Two));
    let best = highscores.0.get(&level.name).copied().unwrap_or(0);
    let text = if two_players {
        format!("P1 {}   :   {} P2", score.points[0], score.points[1])
    } else {
        format!("{}", score.points[0])
    };
    egui::Area::new(egui::Id::new("score"))
        .anchor(egui::Align2::CENTER_TOP, [0.0, 70.0])
        .interactable(false)
        .show(contexts.ctx_mut()?, |ui| {
            ui.vertical_centered(|ui| {
                ui.label(
                    egui::RichText::new(text)
                        .size(32.0)
                        .strong()
                        .color(egui::Color32::WHITE),
                );
                ui.label(egui::RichText::new(format!("highscore {best}")).size(14.0));
            });
        });
    Ok(())
}
