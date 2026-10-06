//! Edit mode: previews of the level's elements, mouse picking and dragging, camera pan/zoom,
//! and the element inspector.

use bevy::camera::ScalingMode;
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::prelude::*;
use bevy::sprite::Anchor;

use crate::level::{Axis, Body, Control, Element, Level};
use crate::materials::Materials;
use crate::play::{CursorWorld, Mode, lattice_spec};
use crate::ui::{pointer_over_ui, typing};
use crate::visuals::WorldCamera;

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
                        edit_pointer.run_if(not(pointer_over_ui)),
                        edit_keys.run_if(not(typing)),
                        camera_controls.run_if(not(pointer_over_ui)),
                        history,
                        rebuild_previews,
                        draw_edit_gizmos,
                    )
                        .chain()
                        .run_if(in_state(Mode::Edit)),
                    sync_view.run_if(resource_changed::<Level>),
                ),
            );
    }
}

fn reset_camera(
    level: Res<Level>,
    mut cameras: Query<(&mut Transform, &mut Projection), With<WorldCamera>>,
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

fn sync_view(level: Res<Level>, mut projections: Query<&mut Projection, With<WorldCamera>>) {
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
        if let Some(tags) = tags(element) {
            let top = element.pos + Vec2::Y * (size.y * 0.5 + 4.0);
            commands.spawn((
                Preview,
                Text2d::new(tags),
                TextFont {
                    font_size: FontSize::Px(11.0),
                    ..default()
                },
                TextColor(Color::srgba(1.0, 1.0, 1.0, 0.65)),
                Anchor::BOTTOM_CENTER,
                Transform::from_translation(top.extend(5.0)),
            ));
        }
    }
}

/// Name and behaviours, shown above an element in the editor.
fn tags(element: &Element) -> Option<String> {
    let mut tags = vec![];
    if !element.name.is_empty() && !matches!(element.body, Body::Wall { .. }) {
        tags.push(element.name.clone());
    }
    if element.control != Control::None {
        tags.push(element.control.name().to_string());
    }
    if element.points != 0 {
        tags.push(format!(
            "{:+} pts to {}",
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
    (!tags.is_empty()).then(|| tags.join(" | "))
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

pub fn duplicate(level: &mut Level, editor: &mut Editor) {
    if let Some(element) = editor.selected.and_then(|i| level.elements.get(i)).cloned() {
        level.elements.push(Element {
            pos: element.pos + Vec2::new(20.0, -20.0),
            ..element
        });
        editor.selected = Some(level.elements.len() - 1);
    }
}

pub fn delete(level: &mut Level, editor: &mut Editor) {
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
    mut cameras: Query<(&mut Transform, &mut Projection), With<WorldCamera>>,
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

pub fn default_body(kind: &str) -> Body {
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
