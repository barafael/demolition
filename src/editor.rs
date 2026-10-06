//! Edit mode: previews of the level's elements, mouse picking and dragging, camera pan/zoom,
//! and the element inspector.

use bevy::camera::ScalingMode;
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::prelude::*;
use bevy_egui::input::{egui_wants_any_keyboard_input, egui_wants_any_pointer_input};
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};

use crate::level::{Axis, Body, Control, Credit, Element, Level, Pins, Player};
use crate::materials::{MaterialKind, Materials};
use crate::play::{CursorWorld, Mode, lattice_spec};

pub struct EditorPlugin;

#[derive(Resource)]
pub struct Editor {
    pub selected: Option<usize>,
    drag: Option<Drag>,
    pan: Option<Vec2>,
    /// Grid step for dragging (0 = free).
    pub snap: f32,
}

impl Default for Editor {
    fn default() -> Self {
        Self {
            selected: None,
            drag: None,
            pan: None,
            snap: 5.0,
        }
    }
}

/// Committed level states for undo/redo. A change is committed once no mouse button is held,
/// so a whole drag or slider gesture is one step.
#[derive(Resource, Default)]
pub struct History {
    undo: Vec<Level>,
    redo: Vec<Level>,
    committed: Option<Level>,
    pub request: Option<HistoryStep>,
}

#[derive(Clone, Copy)]
pub enum HistoryStep {
    Undo,
    Redo,
}

impl History {
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

const HISTORY_LIMIT: usize = 200;

fn history(
    mut level: ResMut<Level>,
    mut history: ResMut<History>,
    mut editor: ResMut<Editor>,
    mouse: Res<ButtonInput<MouseButton>>,
) {
    let history = &mut *history;
    if let Some(step) = history.request.take() {
        let (from, to) = match step {
            HistoryStep::Undo => (&mut history.undo, &mut history.redo),
            HistoryStep::Redo => (&mut history.redo, &mut history.undo),
        };
        if let Some(restored) = from.pop() {
            to.push(level.clone());
            history.committed = Some(restored.clone());
            *level = restored;
            if editor.selected.is_some_and(|i| i >= level.elements.len()) {
                editor.selected = None;
            }
        }
        return;
    }
    let current = level.bypass_change_detection();
    match &history.committed {
        None => history.committed = Some(current.clone()),
        Some(committed)
            if committed != current
                && !mouse.any_pressed([
                    MouseButton::Left,
                    MouseButton::Right,
                    MouseButton::Middle,
                ]) =>
        {
            let previous = history.committed.replace(current.clone()).unwrap();
            history.undo.push(previous);
            if history.undo.len() > HISTORY_LIMIT {
                history.undo.remove(0);
            }
            history.redo.clear();
        }
        Some(_) => {}
    }
}

enum Drag {
    Element { offset: Vec2 },
    Gun { offset: Vec2 },
}

#[derive(Component)]
struct Preview;

impl Plugin for EditorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Editor>()
            .init_resource::<History>()
            .add_systems(OnEnter(Mode::Edit), (reset_camera, despawn_previews))
            .add_systems(OnEnter(Mode::Play), (reset_camera, despawn_previews))
            .add_systems(
                Update,
                (
                    (
                        edit_pointer.run_if(not(egui_wants_any_pointer_input)),
                        edit_keys.run_if(not(egui_wants_any_keyboard_input)),
                        camera_controls.run_if(not(egui_wants_any_pointer_input)),
                        history,
                        rebuild_previews,
                        draw_edit_gizmos,
                    )
                        .chain()
                        .run_if(in_state(Mode::Edit)),
                    sync_view.run_if(resource_changed::<Level>),
                ),
            )
            .add_systems(
                EguiPrimaryContextPass,
                (inspector, labels).run_if(in_state(Mode::Edit)),
            );
    }
}

fn reset_camera(
    level: Res<Level>,
    mut cameras: Query<(&mut Transform, &mut Projection), With<Camera2d>>,
) {
    for (mut transform, mut projection) in &mut cameras {
        transform.translation = Vec3::ZERO;
        *projection = Projection::from(OrthographicProjection {
            scaling_mode: ScalingMode::AutoMin {
                min_width: level.view.x,
                min_height: level.view.y,
            },
            ..OrthographicProjection::default_2d()
        });
    }
}

fn sync_view(level: Res<Level>, mut projections: Query<&mut Projection, With<Camera2d>>) {
    for mut projection in &mut projections {
        if let Projection::Orthographic(ortho) = &mut *projection {
            ortho.scaling_mode = ScalingMode::AutoMin {
                min_width: level.view.x.max(100.0),
                min_height: level.view.y.max(100.0),
            };
        }
    }
}

fn despawn_previews(mut commands: Commands, previews: Query<Entity, With<Preview>>) {
    for entity in &previews {
        commands.entity(entity).despawn();
    }
}

/// Filled shapes for every element, rebuilt whenever the level changes.
fn rebuild_previews(
    mut commands: Commands,
    level: Res<Level>,
    materials: Res<Materials>,
    previews: Query<Entity, With<Preview>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut color_materials: ResMut<Assets<ColorMaterial>>,
) {
    if !level.is_changed() && !materials.is_changed() && !previews.is_empty() {
        return;
    }
    for entity in &previews {
        commands.entity(entity).despawn();
    }
    for (index, element) in level.elements.iter().enumerate() {
        let size = element.size();
        let z = match element.body {
            Body::Wall { .. } => -1.0,
            _ => index as f32 * 0.001,
        };
        let transform = Transform::from_translation(element.pos.extend(z))
            .with_rotation(Quat::from_rotation_z(element.angle));
        let color = match &element.body {
            Body::Lattice { material, .. } => materials.get(*material).color,
            Body::Ball { color, .. } | Body::Wall { color, .. } => Color::srgb_from_array(*color),
        };
        if element.is_round() {
            commands.spawn((
                Preview,
                Mesh2d(meshes.add(Ellipse::new(size.x * 0.5, size.y * 0.5))),
                MeshMaterial2d(color_materials.add(color)),
                transform,
            ));
        } else {
            commands.spawn((Preview, Sprite::from_color(color, size), transform));
        }
    }
}

fn snap(v: Vec2, step: f32) -> Vec2 {
    if step > 0.0 {
        (v / step).round() * step
    } else {
        v
    }
}

fn edit_pointer(
    buttons: Res<ButtonInput<MouseButton>>,
    cursor: Res<CursorWorld>,
    mut level: ResMut<Level>,
    mut editor: ResMut<Editor>,
) {
    let Some(cursor) = cursor.0 else { return };
    if buttons.just_pressed(MouseButton::Left) {
        editor.drag = None;
        if level.gun && cursor.distance(level.gun_pos) < 18.0 {
            editor.drag = Some(Drag::Gun {
                offset: level.gun_pos - cursor,
            });
        } else {
            // Topmost (last drawn) element under the cursor; walls are at the back.
            let hit = level
                .elements
                .iter()
                .enumerate()
                .rev()
                .filter(|(_, e)| e.contains(cursor))
                .min_by_key(|(_, e)| matches!(e.body, Body::Wall { .. }))
                .map(|(i, _)| i);
            editor.selected = hit;
            if let Some(i) = hit {
                editor.drag = Some(Drag::Element {
                    offset: level.elements[i].pos - cursor,
                });
            }
        }
    }
    if buttons.just_released(MouseButton::Left) {
        editor.drag = None;
    }
    if !buttons.pressed(MouseButton::Left) {
        return;
    }
    let step = editor.snap;
    match editor.drag {
        Some(Drag::Gun { offset }) => {
            let pos = snap(cursor + offset, step);
            if level.gun_pos != pos {
                level.gun_pos = pos;
            }
        }
        Some(Drag::Element { offset }) => {
            if let Some(element) = editor.selected.and_then(|i| level.elements.get(i)) {
                let pos = snap(cursor + offset, step);
                if element.pos != pos {
                    let i = editor.selected.unwrap();
                    level.elements[i].pos = pos;
                }
            }
        }
        None => {}
    }
}

fn duplicate(level: &mut Level, editor: &mut Editor) {
    if let Some(element) = editor.selected.and_then(|i| level.elements.get(i)).cloned() {
        level.elements.push(Element {
            pos: element.pos + Vec2::new(20.0, -20.0),
            ..element
        });
        editor.selected = Some(level.elements.len() - 1);
    }
}

fn delete(level: &mut Level, editor: &mut Editor) {
    if let Some(i) = editor.selected.take()
        && i < level.elements.len()
    {
        level.elements.remove(i);
    }
}

fn edit_keys(
    keys: Res<ButtonInput<KeyCode>>,
    mut level: ResMut<Level>,
    mut editor: ResMut<Editor>,
    mut history: ResMut<History>,
) {
    let ctrl = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    if ctrl && keys.just_pressed(KeyCode::KeyZ) {
        history.request = Some(if shift {
            HistoryStep::Redo
        } else {
            HistoryStep::Undo
        });
    }
    if ctrl && keys.just_pressed(KeyCode::KeyY) {
        history.request = Some(HistoryStep::Redo);
    }
    if keys.just_pressed(KeyCode::Escape) {
        editor.selected = None;
    }
    if keys.any_just_pressed([KeyCode::Delete, KeyCode::Backspace]) {
        delete(&mut level, &mut editor);
    }
    if ctrl && keys.just_pressed(KeyCode::KeyD) {
        duplicate(&mut level, &mut editor);
    }
    let moves = [
        KeyCode::KeyQ,
        KeyCode::KeyE,
        KeyCode::ArrowLeft,
        KeyCode::ArrowRight,
        KeyCode::ArrowUp,
        KeyCode::ArrowDown,
    ];
    if !keys.any_just_pressed(moves) {
        return;
    }
    let Some(element) = editor.selected.and_then(|i| level.elements.get_mut(i)) else {
        return;
    };
    let turn = if shift { 1f32 } else { 15f32 }.to_radians();
    if keys.just_pressed(KeyCode::KeyQ) {
        element.angle += turn;
    }
    if keys.just_pressed(KeyCode::KeyE) {
        element.angle -= turn;
    }
    let step = if shift || editor.snap <= 0.0 {
        1.0
    } else {
        editor.snap
    };
    for (key, dir) in [
        (KeyCode::ArrowLeft, Vec2::NEG_X),
        (KeyCode::ArrowRight, Vec2::X),
        (KeyCode::ArrowUp, Vec2::Y),
        (KeyCode::ArrowDown, Vec2::NEG_Y),
    ] {
        if keys.just_pressed(key) {
            element.pos += dir * step;
        }
    }
}

/// Right/middle drag pans, the wheel zooms around the cursor.
fn camera_controls(
    buttons: Res<ButtonInput<MouseButton>>,
    scroll: Res<AccumulatedMouseScroll>,
    cursor: Res<CursorWorld>,
    mut editor: ResMut<Editor>,
    mut cameras: Query<(&mut Transform, &mut Projection), With<Camera2d>>,
) {
    let Some(cursor) = cursor.0 else { return };
    let Ok((mut transform, mut projection)) = cameras.single_mut() else {
        return;
    };
    let panning = buttons.any_pressed([MouseButton::Right, MouseButton::Middle]);
    match (panning, editor.pan) {
        (true, None) => editor.pan = Some(cursor),
        (true, Some(grab)) => transform.translation += (grab - cursor).extend(0.0),
        (false, _) => editor.pan = None,
    }
    if scroll.delta.y != 0.0
        && let Projection::Orthographic(ortho) = &mut *projection
    {
        let factor = 1.15f32.powf(-scroll.delta.y.signum());
        ortho.scale = (ortho.scale * factor).clamp(0.1, 20.0);
        let cam = transform.translation.truncate();
        transform.translation = (cursor + (cam - cursor) * factor).extend(transform.translation.z);
    }
}

fn draw_edit_gizmos(mut gizmos: Gizmos, level: Res<Level>, editor: Res<Editor>) {
    gizmos.rect_2d(
        Isometry2d::IDENTITY,
        level.bounds * 2.0,
        Color::srgba(1.0, 0.3, 0.3, 0.35),
    );
    gizmos.rect_2d(
        Isometry2d::IDENTITY,
        level.view,
        Color::srgba(0.5, 0.7, 1.0, 0.25),
    );

    for (index, element) in level.elements.iter().enumerate() {
        let selected = editor.selected == Some(index);
        let pose = element.pose();
        let size = element.size();
        let outline = if selected {
            Color::srgb(1.0, 0.85, 0.2)
        } else {
            Color::srgba(1.0, 1.0, 1.0, 0.25)
        };
        if element.is_round() {
            gizmos.ellipse_2d(pose, size * 0.5, outline);
        } else {
            gizmos.rect_2d(pose, size, outline);
        }

        let pin_color = Color::srgb(0.3, 1.0, 0.4);
        if let Some(spec) = lattice_spec(element) {
            if !spec.round && (selected || spec.cols * spec.rows <= 400) {
                gizmos.grid_2d(
                    pose,
                    UVec2::new(spec.cols as u32, spec.rows as u32),
                    Vec2::splat(spec.cell),
                    Color::srgba(0.0, 0.0, 0.0, 0.25),
                );
            }
            for pin in &spec.pins {
                let p = pose * (spec.cell_local(pin.col, pin.row) + pin.offset);
                gizmos.circle_2d(p, 2.5, pin_color);
            }
        } else if matches!(element.body, Body::Ball { .. })
            && (element.pins.any() || element.control != Control::None)
        {
            gizmos.circle_2d(element.pos, 2.5, pin_color);
        }

        if element.velocity != Vec2::ZERO {
            gizmos.arrow_2d(
                element.pos,
                element.pos + element.velocity * 0.2,
                Color::srgb(1.0, 0.6, 0.2),
            );
        }
        let cyan = Color::srgb(0.3, 0.9, 1.0);
        match element.control {
            Control::None => {}
            Control::Cursor => {
                gizmos.circle_2d(element.pos, 9.0, cyan);
                gizmos.cross_2d(element.pos, 7.0, cyan);
            }
            Control::Arrows | Control::Wasd
                if element.range > 0.0 && element.axis != Axis::Free =>
            {
                let dir = if element.axis == Axis::Vertical {
                    Vec2::Y
                } else {
                    Vec2::X
                };
                gizmos.line_2d(
                    element.pos - dir * element.range,
                    element.pos + dir * element.range,
                    cyan.with_alpha(0.4),
                );
                gizmos.arrow_2d(element.pos, element.pos + dir * 22.0, cyan);
                gizmos.arrow_2d(element.pos, element.pos - dir * 22.0, cyan);
            }
            Control::Arrows | Control::Wasd => {
                if element.range > 0.0 {
                    gizmos.circle_2d(element.pos, element.range, cyan.with_alpha(0.4));
                }
                let mut dirs = vec![];
                if element.axis != Axis::Vertical {
                    dirs.extend([Vec2::X, Vec2::NEG_X]);
                }
                if element.axis != Axis::Horizontal {
                    dirs.extend([Vec2::Y, Vec2::NEG_Y]);
                }
                for dir in dirs {
                    gizmos.arrow_2d(element.pos, element.pos + dir * 22.0, cyan);
                }
            }
        }
    }

    if level.gun {
        let color = Color::srgb(0.8, 0.8, 0.85);
        gizmos.circle_2d(level.gun_pos, 14.0, color);
        gizmos.line_2d(
            level.gun_pos + Vec2::X * 14.0,
            level.gun_pos + Vec2::X * 36.0,
            color,
        );
    }
}

/// Names and behaviours drawn next to elements.
fn labels(
    mut contexts: EguiContexts,
    level: Res<Level>,
    editor: Res<Editor>,
    cameras: Query<(&Camera, &GlobalTransform)>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    let Ok((camera, camera_transform)) = cameras.single() else {
        return Ok(());
    };
    let painter = ctx.layer_painter(egui::LayerId::background());
    for (index, element) in level.elements.iter().enumerate() {
        let mut tags = vec![];
        let named = editor.selected == Some(index) || !matches!(element.body, Body::Wall { .. });
        if named && !element.name.is_empty() {
            tags.push(element.name.clone());
        }
        if element.control != Control::None {
            tags.push(element.control.name().to_string());
        }
        if element.points != 0 {
            tags.push(format!(
                "★ {:+} to {}",
                element.points,
                element.credit.name()
            ));
        }
        if let Some(owner) = element.owner {
            tags.push(format!("owned by {}", owner.name()));
        }
        if element.respawn {
            tags.push("respawns".into());
        }
        if tags.is_empty() {
            continue;
        }
        let top = element.pos + Vec2::Y * (element.size().y * 0.5 + 4.0);
        let Ok(screen) = camera.world_to_viewport(camera_transform, top.extend(0.0)) else {
            continue;
        };
        painter.text(
            egui::pos2(screen.x, screen.y),
            egui::Align2::CENTER_BOTTOM,
            tags.join(" · "),
            egui::FontId::proportional(11.0),
            egui::Color32::from_white_alpha(170),
        );
    }
    Ok(())
}

fn color_edit(ui: &mut egui::Ui, color: &mut [f32; 3]) {
    ui.horizontal(|ui| {
        ui.label("color");
        ui.color_edit_button_rgb(color);
    });
}

fn default_body(kind: &str) -> Body {
    match kind {
        "Ball" => Body::Ball {
            radius: 12.0,
            density: 0.03,
            restitution: 0.5,
            friction: 0.5,
            color: [0.85, 0.85, 0.9],
        },
        "Wall" => Body::Wall {
            width: 200.0,
            height: 20.0,
            color: [0.22, 0.22, 0.26],
        },
        _ => Element::default().body,
    }
}

pub fn new_element(kind: &str, pos: Vec2) -> Element {
    Element {
        name: kind.to_string(),
        pos,
        body: default_body(kind),
        ..default()
    }
}

fn inspector(
    mut contexts: EguiContexts,
    mut level: ResMut<Level>,
    mut editor: ResMut<Editor>,
    windows: Query<&Window>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    let Some(index) = editor.selected.filter(|&i| i < level.elements.len()) else {
        return Ok(());
    };
    let width = windows.single().map(|w| w.width()).unwrap_or(1600.0);
    // Edit a copy so the level is only marked changed when something actually changed.
    let mut element = level.elements[index].clone();
    let mut action = None;

    egui::Window::new("Inspector")
        .default_pos([width - 330.0, 10.0])
        .default_width(310.0)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .max_height(840.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("name");
                        ui.text_edit_singleline(&mut element.name);
                    });
                    ui.horizontal(|ui| {
                        let current = element.body.kind_name();
                        egui::ComboBox::from_label("kind")
                            .selected_text(current)
                            .show_ui(ui, |ui| {
                                for kind in ["Lattice", "Ball", "Wall"] {
                                    if ui.selectable_label(current == kind, kind).clicked()
                                        && current != kind
                                    {
                                        element.body = default_body(kind);
                                    }
                                }
                            });
                    });
                    ui.horizontal(|ui| {
                        ui.label("pos");
                        ui.add(egui::DragValue::new(&mut element.pos.x).speed(1.0));
                        ui.add(egui::DragValue::new(&mut element.pos.y).speed(1.0));
                    });
                    ui.horizontal(|ui| {
                        ui.label("angle °");
                        let mut degrees = element.angle.to_degrees();
                        if ui
                            .add(egui::DragValue::new(&mut degrees).speed(1.0))
                            .changed()
                        {
                            element.angle = degrees.to_radians();
                        }
                    });

                    ui.separator();
                    match &mut element.body {
                        Body::Lattice {
                            material,
                            cols,
                            rows,
                            cell,
                            round,
                        } => {
                            egui::ComboBox::from_label("material")
                                .selected_text(material.name())
                                .show_ui(ui, |ui| {
                                    for kind in MaterialKind::ALL {
                                        ui.selectable_value(material, kind, kind.name());
                                    }
                                });
                            ui.horizontal(|ui| {
                                ui.label("cells");
                                ui.add(egui::DragValue::new(cols).range(1..=200).speed(0.2));
                                ui.label("×");
                                ui.add(egui::DragValue::new(rows).range(1..=200).speed(0.2));
                            });
                            ui.add(egui::Slider::new(cell, 3.0..=40.0).text("cell size"));
                            ui.checkbox(round, "round");
                            ui.small(format!("{} cells", *cols * *rows));
                        }
                        Body::Ball {
                            radius,
                            density,
                            restitution,
                            friction,
                            color,
                        } => {
                            ui.add(egui::Slider::new(radius, 1.0..=100.0).text("radius"));
                            ui.add(
                                egui::Slider::new(density, 0.001..=0.2)
                                    .logarithmic(true)
                                    .text("density"),
                            );
                            ui.add(egui::Slider::new(restitution, 0.0..=1.0).text("restitution"));
                            ui.add(egui::Slider::new(friction, 0.0..=1.5).text("friction"));
                            color_edit(ui, color);
                        }
                        Body::Wall {
                            width,
                            height,
                            color,
                        } => {
                            ui.add(
                                egui::DragValue::new(width)
                                    .range(1.0..=10000.0)
                                    .prefix("w "),
                            );
                            ui.add(
                                egui::DragValue::new(height)
                                    .range(1.0..=10000.0)
                                    .prefix("h "),
                            );
                            color_edit(ui, color);
                        }
                    }

                    if !matches!(element.body, Body::Wall { .. }) {
                        ui.separator();
                        ui.label("Pins");
                        let pins: &mut Pins = &mut element.pins;
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut pins.left, "left");
                            ui.checkbox(&mut pins.right, "right");
                            ui.checkbox(&mut pins.top, "top");
                            ui.checkbox(&mut pins.bottom, "bottom");
                        });
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut pins.center, "center");
                            ui.add(
                                egui::DragValue::new(&mut pins.every)
                                    .range(1..=100)
                                    .prefix("every "),
                            );
                        });
                        ui.horizontal(|ui| {
                            ui.label("initial velocity");
                            ui.add(egui::DragValue::new(&mut element.velocity.x).speed(5.0));
                            ui.add(egui::DragValue::new(&mut element.velocity.y).speed(5.0));
                        });
                    }

                    ui.separator();
                    ui.label("Control");
                    egui::ComboBox::from_label("owner")
                        .selected_text(element.owner.map_or("nobody", Player::name))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut element.owner, None, "nobody");
                            for player in [Player::One, Player::Two] {
                                ui.selectable_value(
                                    &mut element.owner,
                                    Some(player),
                                    player.name(),
                                );
                            }
                        });
                    ui.small("What an owned element hits counts as hit by its owner.");
                    egui::ComboBox::from_label("input")
                        .selected_text(element.control.name())
                        .show_ui(ui, |ui| {
                            for control in Control::ALL {
                                ui.selectable_value(&mut element.control, control, control.name());
                            }
                        });
                    if element.control != Control::None {
                        egui::ComboBox::from_label("axis")
                            .selected_text(element.axis.name())
                            .show_ui(ui, |ui| {
                                for axis in Axis::ALL {
                                    ui.selectable_value(&mut element.axis, axis, axis.name());
                                }
                            });
                        ui.add(
                            egui::Slider::new(&mut element.speed, 50.0..=3000.0).text("max speed"),
                        );
                        ui.add(
                            egui::Slider::new(&mut element.range, 0.0..=2000.0)
                                .text("range (0 = any)"),
                        );
                        if !matches!(element.body, Body::Wall { .. }) {
                            ui.small(
                                "Hangs from an invisible carrier by its pins (center if none).",
                            );
                        }
                    }

                    if !matches!(element.body, Body::Wall { .. }) {
                        ui.separator();
                        ui.label("Destruction");
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::DragValue::new(&mut element.points)
                                    .speed(10.0)
                                    .prefix("points "),
                            );
                            egui::ComboBox::from_id_salt("credit")
                                .selected_text(format!("to {}", element.credit.name()))
                                .show_ui(ui, |ui| {
                                    for credit in Credit::ALL {
                                        ui.selectable_value(
                                            &mut element.credit,
                                            credit,
                                            credit.name(),
                                        );
                                    }
                                });
                        });
                        if matches!(element.body, Body::Lattice { .. }) {
                            ui.add(
                                egui::Slider::new(&mut element.destroyed_at, 0.0..=1.0)
                                    .text("destroyed at bond loss"),
                            );
                        }
                        ui.small(
                            "Losing all pins or leaving the level bounds also counts as destroyed.",
                        );
                        ui.checkbox(&mut element.respawn, "respawn when destroyed");
                        ui.add(
                            egui::Slider::new(&mut element.keep_speed, 0.0..=3000.0)
                                .text("keep speed"),
                        );
                        if element.keep_speed > 0.0 {
                            ui.add(
                                egui::Slider::new(&mut element.speed_ramp, 0.0..=200.0)
                                    .text("speed ramp /s"),
                            );
                        }
                        let mut bouncy = element.bounce.is_some();
                        ui.horizontal(|ui| {
                            if ui.checkbox(&mut bouncy, "bounce override").changed() {
                                element.bounce = bouncy.then_some(1.0);
                            }
                            if let Some(bounce) = &mut element.bounce {
                                ui.add(egui::Slider::new(bounce, 0.0..=1.0));
                            }
                        });
                        let mut slippery = element.friction.is_some();
                        ui.horizontal(|ui| {
                            if ui.checkbox(&mut slippery, "friction override").changed() {
                                element.friction = slippery.then_some(0.0);
                            }
                            if let Some(friction) = &mut element.friction {
                                ui.add(egui::Slider::new(friction, 0.0..=1.5));
                            }
                        });
                    }

                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui.button("Duplicate (Ctrl+D)").clicked() {
                            action = Some("duplicate");
                        }
                        if ui.button("Delete (Del)").clicked() {
                            action = Some("delete");
                        }
                    });
                });
        });

    if element != level.elements[index] {
        level.elements[index] = element;
    }
    match action {
        Some("duplicate") => duplicate(&mut level, &mut editor),
        Some("delete") => delete(&mut level, &mut editor),
        _ => {}
    }
    Ok(())
}
