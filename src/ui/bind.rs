//! Two-way bindings between widgets and app state.
//!
//! A widget carries `Bind(field)`. Every frame, the sync systems push the field's current value
//! into the widget (unless the user is editing it), and global observers write the user's edits
//! back. `Model` is the single place that knows how to read and write each field.

use bevy::ecs::system::SystemParam;
use bevy::feathers::controls::{NumberInputValue, UpdateNumberInput};
use bevy::input_focus::InputFocus;
use bevy::prelude::*;
use bevy::text::{EditableText, TextEdit, TextEditChange};
use bevy::ui::Checked;
use bevy::ui_widgets::{SliderValue, ValueChange};

use crate::editor::Editor;
use crate::gun::{Ammo, Gun};
use crate::level::{Axis, Body, Control, Credit, Element, Level, Player};
use crate::materials::{MaterialKind, Strength};
use crate::visuals::View;

/// A value a widget edits.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Field {
    #[default]
    None,
    LevelName,
    Gravity,
    Substeps,
    ViewWidth,
    ViewHeight,
    BoundsWidth,
    BoundsHeight,
    GunEnabled,
    Snap,
    ShowGrid,
    Ammo,
    GunSpeed,
    AutoFire,
    TimeScale,
    Paused,
    StressOverlay,
    Trajectory,
    ImpactFx,
    StressGlow,
    /// A material parameter; `None` is the pins.
    Material(Option<MaterialKind>, MatParam),
    /// A property of the selected element.
    Element(ElementField),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MatParam {
    Explosive,
    Density,
    Friction,
    Restitution,
    PointCompliance,
    AngleCompliance,
    YieldStrain,
    YieldAngle,
    Ductility,
    BreakStrain,
    BreakAngle,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ElementField {
    Name,
    Kind,
    PosX,
    PosY,
    Angle,
    VelX,
    VelY,
    Material,
    Cols,
    Rows,
    Cell,
    Round,
    Radius,
    Density,
    Restitution,
    Friction,
    Width,
    Height,
    Red,
    Green,
    Blue,
    PinLeft,
    PinRight,
    PinTop,
    PinBottom,
    PinCenter,
    PinEvery,
    Control,
    Axis,
    Speed,
    Range,
    Owner,
    Points,
    Credit,
    DestroyedAt,
    Respawn,
    KeepSpeed,
    SpeedRamp,
    BounceOverride,
    Bounce,
    FrictionOverride,
    FrictionValue,
}

pub const KINDS: [&str; 3] = ["Lattice", "Ball", "Wall"];
const OWNERS: [Option<Player>; 3] = [None, Some(Player::One), Some(Player::Two)];

impl Field {
    /// Option names for fields edited with a choice menu.
    pub fn choices(self) -> Vec<&'static str> {
        use ElementField as E;
        match self {
            Field::Ammo => Ammo::ALL.iter().map(|a| a.name()).collect(),
            Field::Element(E::Kind) => KINDS.to_vec(),
            Field::Element(E::Material) => MaterialKind::ALL.iter().map(|m| m.name()).collect(),
            Field::Element(E::Control) => Control::ALL.iter().map(|c| c.name()).collect(),
            Field::Element(E::Axis) => Axis::ALL.iter().map(|a| a.name()).collect(),
            Field::Element(E::Owner) => vec!["nobody", "P1", "P2"],
            Field::Element(E::Credit) => Credit::ALL.iter().map(|c| c.name()).collect(),
            _ => vec![],
        }
    }

    /// The name of option `index`, without building the whole list.
    pub fn choice_name(self, index: usize) -> Option<&'static str> {
        use ElementField as E;
        match self {
            Field::Ammo => Ammo::ALL.get(index).map(|a| a.name()),
            Field::Element(E::Kind) => KINDS.get(index).copied(),
            Field::Element(E::Material) => MaterialKind::ALL.get(index).map(|m| m.name()),
            Field::Element(E::Control) => Control::ALL.get(index).map(|c| c.name()),
            Field::Element(E::Axis) => Axis::ALL.get(index).map(|a| a.name()),
            Field::Element(E::Owner) => ["nobody", "P1", "P2"].get(index).copied(),
            Field::Element(E::Credit) => Credit::ALL.get(index).map(|c| c.name()),
            _ => None,
        }
    }

    /// Compliances span many orders of magnitude, so their sliders edit log10 of the value.
    pub fn is_log(self) -> bool {
        matches!(
            self,
            Field::Material(_, MatParam::PointCompliance | MatParam::AngleCompliance)
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    F(f32),
    B(bool),
    C(usize),
    S(String),
}

/// The widget's bound field.
#[derive(Component, Clone, Copy, Default)]
pub struct Bind(pub Field);

/// Marks the caption text of a choice menu.
#[derive(Component, Clone, Copy, Default)]
pub struct ChoiceCaption;

/// A choice menu item: picking it sets `field` to option `index`.
#[derive(Component, Clone, Copy, Default)]
pub struct ChoiceItem {
    pub field: Field,
    pub index: usize,
}

/// Shows the node only while `field` is on: true, non-zero, or a choice other than the first.
#[derive(Component, Clone, Copy, Default)]
pub struct ShowWhen(pub Field);

/// The value last pushed into a number input, so it is only updated when the model changes.
#[derive(Component, Clone, Copy, Default)]
pub struct Shown(pub Option<f32>);

#[derive(SystemParam)]
pub struct Model<'w> {
    pub level: ResMut<'w, Level>,
    pub editor: ResMut<'w, Editor>,
    pub gun: ResMut<'w, Gun>,
    pub view: ResMut<'w, View>,
    pub time: ResMut<'w, Time<Virtual>>,
}

fn strength(s: &Strength, p: MatParam) -> f32 {
    match p {
        MatParam::PointCompliance => s.point_compliance,
        MatParam::AngleCompliance => s.angle_compliance,
        MatParam::YieldStrain => s.yield_strain,
        MatParam::YieldAngle => s.yield_angle,
        MatParam::Ductility => s.ductility,
        MatParam::BreakStrain => s.break_strain,
        MatParam::BreakAngle => s.break_angle,
        MatParam::Density | MatParam::Friction | MatParam::Restitution | MatParam::Explosive => 0.0,
    }
}

fn strength_mut(s: &mut Strength, p: MatParam) -> Option<&mut f32> {
    Some(match p {
        MatParam::PointCompliance => &mut s.point_compliance,
        MatParam::AngleCompliance => &mut s.angle_compliance,
        MatParam::YieldStrain => &mut s.yield_strain,
        MatParam::YieldAngle => &mut s.yield_angle,
        MatParam::Ductility => &mut s.ductility,
        MatParam::BreakStrain => &mut s.break_strain,
        MatParam::BreakAngle => &mut s.break_angle,
        MatParam::Density | MatParam::Friction | MatParam::Restitution | MatParam::Explosive => {
            return None;
        }
    })
}

fn element_get(e: &Element, f: ElementField) -> Option<Value> {
    use ElementField as E;
    use Value::{B, C, F, S};
    let pins = &e.pins;
    Some(match f {
        E::Name => S(e.name.clone()),
        E::Kind => C(KINDS.iter().position(|k| *k == e.body.kind_name())?),
        E::PosX => F(e.pos.x),
        E::PosY => F(e.pos.y),
        E::Angle => F(e.angle.to_degrees()),
        E::VelX => F(e.velocity.x),
        E::VelY => F(e.velocity.y),
        E::PinLeft => B(pins.left),
        E::PinRight => B(pins.right),
        E::PinTop => B(pins.top),
        E::PinBottom => B(pins.bottom),
        E::PinCenter => B(pins.center),
        E::PinEvery => F(pins.every as f32),
        E::Control => C(Control::ALL.iter().position(|c| *c == e.control)?),
        E::Axis => C(Axis::ALL.iter().position(|a| *a == e.axis)?),
        E::Speed => F(e.speed),
        E::Range => F(e.range),
        E::Owner => C(OWNERS.iter().position(|o| *o == e.owner)?),
        E::Points => F(e.points as f32),
        E::Credit => C(Credit::ALL.iter().position(|c| *c == e.credit)?),
        E::DestroyedAt => F(e.destroyed_at),
        E::Respawn => B(e.respawn),
        E::KeepSpeed => F(e.keep_speed),
        E::SpeedRamp => F(e.speed_ramp),
        E::BounceOverride => B(e.bounce.is_some()),
        E::Bounce => F(e.bounce?),
        E::FrictionOverride => B(e.friction.is_some()),
        E::FrictionValue => F(e.friction?),
        _ => match (&e.body, f) {
            (Body::Lattice { material, .. }, E::Material) => {
                C(MaterialKind::ALL.iter().position(|m| m == material)?)
            }
            (Body::Lattice { cols, .. }, E::Cols) => F(*cols as f32),
            (Body::Lattice { rows, .. }, E::Rows) => F(*rows as f32),
            (Body::Lattice { cell, .. }, E::Cell) => F(*cell),
            (Body::Lattice { round, .. }, E::Round) => B(*round),
            (Body::Ball { radius, .. }, E::Radius) => F(*radius),
            (Body::Ball { density, .. }, E::Density) => F(*density),
            (Body::Ball { restitution, .. }, E::Restitution) => F(*restitution),
            (Body::Ball { friction, .. }, E::Friction) => F(*friction),
            (Body::Wall { width, .. }, E::Width) => F(*width),
            (Body::Wall { height, .. }, E::Height) => F(*height),
            (Body::Ball { color, .. } | Body::Wall { color, .. }, E::Red) => F(color[0]),
            (Body::Ball { color, .. } | Body::Wall { color, .. }, E::Green) => F(color[1]),
            (Body::Ball { color, .. } | Body::Wall { color, .. }, E::Blue) => F(color[2]),
            _ => return None,
        },
    })
}

fn element_set(e: &mut Element, f: ElementField, v: Value) {
    use ElementField as E;
    let (num, flag, choice) = match &v {
        Value::F(x) => (*x, false, 0),
        Value::B(b) => (0.0, *b, 0),
        Value::C(i) => (0.0, false, *i),
        Value::S(s) => {
            if f == E::Name {
                e.name = s.clone();
            }
            return;
        }
    };
    let count = |x: f32| x.round().clamp(1.0, 400.0) as i32;
    match f {
        E::Name => {}
        E::Kind => {
            if let Some(kind) = KINDS.get(choice)
                && *kind != e.body.kind_name()
            {
                e.body = crate::editor::default_body(kind);
            }
        }
        E::PosX => e.pos.x = num,
        E::PosY => e.pos.y = num,
        E::Angle => e.angle = num.to_radians(),
        E::VelX => e.velocity.x = num,
        E::VelY => e.velocity.y = num,
        E::PinLeft => e.pins.left = flag,
        E::PinRight => e.pins.right = flag,
        E::PinTop => e.pins.top = flag,
        E::PinBottom => e.pins.bottom = flag,
        E::PinCenter => e.pins.center = flag,
        E::PinEvery => e.pins.every = num.round().clamp(1.0, 100.0) as u32,
        E::Control => e.control = Control::ALL.get(choice).copied().unwrap_or_default(),
        E::Axis => e.axis = Axis::ALL.get(choice).copied().unwrap_or_default(),
        E::Speed => e.speed = num,
        E::Range => e.range = num.max(0.0),
        E::Owner => e.owner = OWNERS.get(choice).copied().flatten(),
        E::Points => e.points = num.round() as i32,
        E::Credit => e.credit = Credit::ALL.get(choice).copied().unwrap_or_default(),
        E::DestroyedAt => e.destroyed_at = num,
        E::Respawn => e.respawn = flag,
        E::KeepSpeed => e.keep_speed = num,
        E::SpeedRamp => e.speed_ramp = num,
        E::BounceOverride => e.bounce = flag.then_some(e.bounce.unwrap_or(1.0)),
        E::Bounce => e.bounce = Some(num),
        E::FrictionOverride => e.friction = flag.then_some(e.friction.unwrap_or(0.0)),
        E::FrictionValue => e.friction = Some(num),
        _ => match (&mut e.body, f) {
            (Body::Lattice { material, .. }, E::Material) => {
                *material = MaterialKind::ALL.get(choice).copied().unwrap_or(*material);
            }
            (Body::Lattice { cols, .. }, E::Cols) => *cols = count(num),
            (Body::Lattice { rows, .. }, E::Rows) => *rows = count(num),
            (Body::Lattice { cell, .. }, E::Cell) => *cell = num.max(1.0),
            (Body::Lattice { round, .. }, E::Round) => *round = flag,
            (Body::Ball { radius, .. }, E::Radius) => *radius = num.max(1.0),
            (Body::Ball { density, .. }, E::Density) => *density = num.max(1e-4),
            (Body::Ball { restitution, .. }, E::Restitution) => *restitution = num,
            (Body::Ball { friction, .. }, E::Friction) => *friction = num,
            (Body::Wall { width, .. }, E::Width) => *width = num.max(1.0),
            (Body::Wall { height, .. }, E::Height) => *height = num.max(1.0),
            (Body::Ball { color, .. } | Body::Wall { color, .. }, E::Red) => color[0] = num,
            (Body::Ball { color, .. } | Body::Wall { color, .. }, E::Green) => color[1] = num,
            (Body::Ball { color, .. } | Body::Wall { color, .. }, E::Blue) => color[2] = num,
            _ => {}
        },
    }
}

impl Model<'_> {
    /// Reads a field without marking anything changed.
    pub fn get(&mut self, field: Field) -> Option<Value> {
        use Value::{B, C, F, S};
        let level = self.level.bypass_change_detection();
        let gun = self.gun.bypass_change_detection();
        let view = self.view.bypass_change_detection();
        let materials = &level.materials;
        Some(match field {
            Field::None => return None,
            Field::LevelName => S(level.name.clone()),
            Field::Gravity => F(level.gravity),
            Field::Substeps => F(level.substeps as f32),
            Field::ViewWidth => F(level.view.x),
            Field::ViewHeight => F(level.view.y),
            Field::BoundsWidth => F(level.bounds.x),
            Field::BoundsHeight => F(level.bounds.y),
            Field::GunEnabled => B(level.gun),
            Field::Snap => F(self.editor.bypass_change_detection().snap),
            Field::ShowGrid => B(self.editor.bypass_change_detection().show_grid),
            Field::Ammo => C(Ammo::ALL.iter().position(|a| *a == gun.ammo)?),
            Field::GunSpeed => F(gun.speed),
            Field::AutoFire => F(gun.auto_rate),
            Field::TimeScale => F(self.time.relative_speed()),
            Field::Paused => B(self.time.is_paused()),
            Field::StressOverlay => B(view.stress_overlay),
            Field::Trajectory => B(view.trajectory),
            Field::ImpactFx => B(view.impact_fx),
            Field::StressGlow => B(view.stress_glow),
            Field::Material(kind, param) => F(match (kind, param) {
                (Some(k), MatParam::Density) => materials.get(k).density,
                (Some(k), MatParam::Friction) => materials.get(k).friction,
                (Some(k), MatParam::Restitution) => materials.get(k).restitution,
                (Some(k), MatParam::Explosive) => materials.get(k).explosive,
                (Some(k), p) => strength(&materials.get(k).strength, p),
                (None, p) => strength(&materials.pins, p),
            }),
            Field::Element(f) => {
                let index = self.editor.bypass_change_detection().selected?;
                element_get(level.elements.get(index)?, f)?
            }
        })
    }

    /// Writes a field, marking resources changed only if the value actually differs.
    pub fn set(&mut self, field: Field, value: Value) {
        if self.get(field).as_ref() == Some(&value) {
            return;
        }
        let num = match value {
            Value::F(x) => x,
            _ => 0.0,
        };
        let flag = matches!(value, Value::B(true));
        match field {
            Field::None => {}
            Field::LevelName => {
                if let Value::S(name) = value {
                    self.level.name = name;
                }
            }
            Field::Gravity => self.level.gravity = num,
            Field::Substeps => self.level.substeps = num.round().clamp(1.0, 100.0) as u32,
            Field::ViewWidth => self.level.view.x = num.max(100.0),
            Field::ViewHeight => self.level.view.y = num.max(100.0),
            Field::BoundsWidth => self.level.bounds.x = num.max(100.0),
            Field::BoundsHeight => self.level.bounds.y = num.max(100.0),
            Field::GunEnabled => self.level.gun = flag,
            Field::Snap => self.editor.snap = num.max(0.0),
            Field::ShowGrid => self.editor.show_grid = flag,
            Field::Ammo => {
                if let Value::C(i) = value
                    && let Some(&ammo) = Ammo::ALL.get(i)
                {
                    self.gun.ammo = ammo;
                    self.gun.speed = ammo.default_speed();
                }
            }
            Field::GunSpeed => self.gun.speed = num,
            Field::AutoFire => self.gun.auto_rate = num,
            Field::TimeScale => self.time.set_relative_speed(num.max(0.01)),
            Field::Paused => {
                if flag {
                    self.time.pause();
                } else {
                    self.time.unpause();
                }
            }
            Field::StressOverlay => self.view.stress_overlay = flag,
            Field::Trajectory => self.view.trajectory = flag,
            Field::ImpactFx => self.view.impact_fx = flag,
            Field::StressGlow => self.view.stress_glow = flag,
            Field::Material(kind, param) => {
                let materials = &mut self.level.materials;
                let slot = match (kind, param) {
                    (Some(k), MatParam::Density) => Some(&mut materials.table[k as usize].density),
                    (Some(k), MatParam::Friction) => {
                        Some(&mut materials.table[k as usize].friction)
                    }
                    (Some(k), MatParam::Restitution) => {
                        Some(&mut materials.table[k as usize].restitution)
                    }
                    (Some(k), MatParam::Explosive) => {
                        Some(&mut materials.table[k as usize].explosive)
                    }
                    (Some(k), p) => strength_mut(&mut materials.table[k as usize].strength, p),
                    (None, p) => strength_mut(&mut materials.pins, p),
                };
                if let Some(slot) = slot {
                    *slot = num;
                }
            }
            Field::Element(f) => {
                let Some(index) = self.editor.selected else {
                    return;
                };
                if let Some(element) = self.level.elements.get_mut(index) {
                    element_set(element, f, value);
                }
            }
        }
    }
}

fn to_widget(field: Field, value: f32) -> f32 {
    if field.is_log() {
        value.max(1e-12).log10()
    } else {
        value
    }
}

fn from_widget(field: Field, value: f32) -> f32 {
    if field.is_log() {
        10f32.powf(value)
    } else {
        value
    }
}

/// Slider or number input edited.
pub fn on_f32_change(
    change: On<ValueChange<f32>>,
    binds: Query<&Bind>,
    mut shown: Query<&mut Shown>,
    focus: Res<InputFocus>,
    parents: Query<&ChildOf>,
    mut model: Model,
) {
    let Ok(&Bind(field)) = binds.get(change.source) else {
        return;
    };
    if let Ok(mut shown) = shown.get_mut(change.source) {
        // A number input. It also reports the values `sync_widgets` pushes into it, which may
        // be stale by the time they arrive, so only keystrokes in its own (focused) text field
        // and the final value of an edit count.
        let typing_here = focus
            .get()
            .and_then(|f| parents.get(f).ok())
            .is_some_and(|parent| parent.parent() == change.source);
        if !typing_here && !change.is_final {
            return;
        }
        if change.is_final {
            // Re-sync after the edit, so a clamped or rejected entry shows the real value.
            shown.0 = None;
        }
    }
    let value = from_widget(field, change.value);
    model.set(field, Value::F(value));
}

/// Checkbox or toggle switch flipped.
pub fn on_bool_change(
    change: On<ValueChange<bool>>,
    binds: Query<&Bind>,
    mut model: Model,
    mut commands: Commands,
) {
    let Ok(&Bind(field)) = binds.get(change.source) else {
        return;
    };
    model.set(field, Value::B(change.value));
    let mut entity = commands.entity(change.source);
    if change.value {
        entity.insert(Checked);
    } else {
        entity.remove::<Checked>();
    }
}

/// Choice menu item picked.
pub fn on_choice(
    activate: On<bevy::ui_widgets::Activate>,
    items: Query<&ChoiceItem>,
    mut model: Model,
) {
    if let Ok(item) = items.get(activate.entity) {
        model.set(item.field, Value::C(item.index));
    }
}

/// Text input edited. Only edits made while the field has focus count: a freshly spawned field
/// reports its initial empty text too, which must not wipe the name it is about to show.
pub fn on_text_change(
    change: On<TextEditChange>,
    texts: Query<(&Bind, &EditableText)>,
    focus: Res<InputFocus>,
    mut model: Model,
) {
    if focus.get() != Some(change.event_target()) {
        return;
    }
    if let Ok((&Bind(field), text)) = texts.get(change.event_target()) {
        model.set(field, Value::S(text.value().to_string()));
    }
}

/// Pushes model values into widgets that show something different.
pub fn sync_widgets(
    mut model: Model,
    focus: Res<InputFocus>,
    mut sliders: Query<(Entity, &Bind, &SliderValue)>,
    mut numbers: Query<(Entity, &Bind, &mut Shown, &Children)>,
    checks: Query<(Entity, &Bind, Has<Checked>), With<bevy::ui_widgets::Checkbox>>,
    mut captions: Query<(&Bind, &mut Text), With<ChoiceCaption>>,
    mut texts: Query<(Entity, &Bind, &mut EditableText)>,
    mut commands: Commands,
) {
    for (entity, &Bind(field), slider) in &mut sliders {
        if let Some(Value::F(v)) = model.get(field) {
            let v = to_widget(field, v);
            if (v - slider.0).abs() > 1e-6 * v.abs().max(1.0) {
                commands.entity(entity).insert(SliderValue(v));
            }
        }
    }
    for (entity, &Bind(field), mut shown, children) in &mut numbers {
        let Some(Value::F(v)) = model.get(field) else {
            continue;
        };
        let editing = children.iter().any(|c| focus.get() == Some(c));
        if shown.0 != Some(v) && !editing {
            shown.0 = Some(v);
            commands.trigger(UpdateNumberInput {
                entity,
                value: NumberInputValue::F32(v),
            });
        }
    }
    for (entity, &Bind(field), checked) in &checks {
        if let Some(Value::B(b)) = model.get(field)
            && b != checked
        {
            if b {
                commands.entity(entity).insert(Checked);
            } else {
                commands.entity(entity).remove::<Checked>();
            }
        }
    }
    for (&Bind(field), mut text) in &mut captions {
        if let Some(Value::C(i)) = model.get(field)
            && let Some(name) = field.choice_name(i)
            && text.0 != name
        {
            text.0 = name.to_string();
        }
    }
    for (entity, &Bind(field), mut editable) in &mut texts {
        if let Some(Value::S(s)) = model.get(field)
            && focus.get() != Some(entity)
            && editable.value() != s.as_str()
        {
            editable.queue_edit(TextEdit::SelectAll);
            editable.queue_edit(TextEdit::Insert(s.into()));
        }
    }
}

/// Shows or hides `ShowWhen` nodes. Hiding rather than rebuilding keeps a slider alive while it
/// is being dragged across the value that toggles its neighbours.
pub fn show_when(mut model: Model, mut nodes: Query<(&ShowWhen, &mut Node)>) {
    for (&ShowWhen(field), mut node) in &mut nodes {
        let on = match model.get(field) {
            Some(Value::B(b)) => b,
            Some(Value::F(x)) => x > 0.0,
            Some(Value::C(i)) => i > 0,
            Some(Value::S(s)) => !s.is_empty(),
            None => false,
        };
        let display = if on { Display::Flex } else { Display::None };
        if node.display != display {
            node.display = display;
        }
    }
}
