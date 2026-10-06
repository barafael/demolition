//! Bevy UI (feathers widgets): a sidebar on the left for level, tools and settings, a
//! collapsible toolbar on the right for tweaking the selected element, and the score HUD.

mod bind;
mod inspector;
mod sidebar;
mod widgets;

use std::collections::HashSet;

use avian2d::prelude::RigidBody;
use bevy::clipboard::{Clipboard, ClipboardRead};
use bevy::ecs::VariantDefaults;
use bevy::feathers::FeathersPlugins;
use bevy::feathers::controls::ButtonVariant;
use bevy::feathers::dark_theme::create_dark_theme;
use bevy::feathers::theme::UiTheme;
use bevy::input_focus::InputFocus;
use bevy::picking::hover::HoverMap;
use bevy::prelude::*;
use bevy::text::EditableText;
use bevy::ui::InteractionDisabled;
use bevy::ui_widgets::{Activate, MenuItem, MenuPopup};

use crate::editor::{self, Editor, History, HistoryStep};
use crate::fracture::Stats;
use crate::level::{self, Credit, Highscores, Level, Player};
use crate::materials::MaterialKind;
use crate::play::{ClearDebris, Mode, Restart, Score};

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(FeathersPlugins)
            .insert_resource(UiTheme(create_dark_theme()))
            .init_resource::<OpenSections>()
            .init_resource::<ToolbarOpen>()
            .init_resource::<LevelFiles>()
            .init_resource::<Actions>()
            .init_resource::<PendingPaste>()
            .add_observer(on_activate)
            .add_observer(bind::on_f32_change)
            .add_observer(bind::on_bool_change)
            .add_observer(bind::on_choice)
            .add_observer(bind::on_text_change)
            .add_systems(
                Startup,
                (sidebar::spawn, inspector::spawn_toolbar, spawn_hud),
            )
            .add_systems(
                Update,
                (
                    release_focus,
                    apply_actions,
                    poll_paste,
                    bind::sync_widgets,
                    show_sections,
                    show_for_mode,
                    sidebar::rebuild_lists,
                    inspector::rebuild,
                    update_texts,
                    enable_history_buttons,
                )
                    .chain(),
            );
    }
}

pub const SIDEBAR_WIDTH: f32 = sidebar::WIDTH;
/// Width of the toolbar when open: the inspector plus its toggle button column.
pub const TOOLBAR_WIDTH: f32 = inspector::WIDTH + TOOLBAR_COLLAPSED_WIDTH;
pub const TOOLBAR_COLLAPSED_WIDTH: f32 = 44.0;

/// Run condition: the pointer is over a panel, so clicks and scrolls aren't for the world.
pub fn pointer_over_ui(hover: Res<HoverMap>, nodes: Query<(), With<Node>>) -> bool {
    hover
        .values()
        .any(|hits| hits.keys().any(|entity| nodes.contains(*entity)))
}

/// Run condition: a text field has keyboard focus, so keys aren't shortcuts.
pub fn typing(focus: Res<InputFocus>, texts: Query<(), With<EditableText>>) -> bool {
    focus.get().is_some_and(|entity| texts.contains(entity))
}

/// The accordion sections of the sidebar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Section {
    #[default]
    World,
    Elements,
    Gun,
    View,
    Materials,
    Material(Option<MaterialKind>),
    Help,
}

#[derive(Resource)]
pub struct OpenSections(HashSet<Section>);

impl Default for OpenSections {
    fn default() -> Self {
        Self(HashSet::from([
            Section::Elements,
            Section::Gun,
            Section::View,
        ]))
    }
}

/// Whether the right-hand toolbar is expanded.
#[derive(Resource)]
pub struct ToolbarOpen(pub bool);

impl Default for ToolbarOpen {
    fn default() -> Self {
        Self(true)
    }
}

#[derive(Component, Clone, Copy, Default)]
pub struct SectionBody(pub Section);

/// One of the two chevrons of a section header; shown when the section's state matches `open`.
#[derive(Component, Clone, Copy, Default)]
pub struct Chevron {
    pub section: Section,
    pub open: bool,
}

/// Shown only in this mode.
#[derive(Component, Clone, Copy, Default)]
pub struct OnlyIn(pub Mode);

/// Text that is regenerated every frame.
#[derive(Component, Clone, Copy, Default, PartialEq, VariantDefaults)]
pub enum Dyn {
    #[default]
    Stats,
    Status,
    PlayToggle,
    Score,
    Highscore,
    ToolbarToggle,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Action {
    #[default]
    None,
    TogglePlay,
    Restart,
    ClearDebris,
    Undo,
    Redo,
    Add(&'static str),
    Duplicate,
    Delete,
    Select(usize),
    Save,
    Load(String),
    Preset(&'static str),
    CopyLevel,
    PasteLevel,
    ToggleSection(Section),
    ToggleToolbar,
}

/// What a button does when activated.
#[derive(Component, Clone, Default)]
pub struct Act(pub Action);

/// Activated buttons queue their action here; `apply_actions` carries them out.
#[derive(Resource, Default)]
struct Actions(Vec<Action>);

/// Saved level names, refreshed after saving, and the last file operation's outcome.
#[derive(Resource)]
pub struct LevelFiles {
    pub names: Vec<String>,
    pub status: String,
}

impl Default for LevelFiles {
    fn default() -> Self {
        Self {
            names: level::list(),
            status: String::new(),
        }
    }
}

/// A clipboard read in flight (asynchronous in the browser).
#[derive(Resource, Default)]
struct PendingPaste(Option<ClipboardRead>);

fn on_activate(activate: On<Activate>, acts: Query<&Act>, mut actions: ResMut<Actions>) {
    if let Ok(act) = acts.get(activate.entity) {
        actions.0.push(act.0.clone());
    }
}

/// Buttons and checkboxes keep keyboard focus after a click, which would make Space, Enter
/// and the arrow keys operate them instead of the game. Focus is only kept for text fields and
/// open menus.
fn release_focus(
    mut focus: ResMut<InputFocus>,
    keep: Query<(), Or<(With<EditableText>, With<MenuItem>, With<MenuPopup>)>>,
    parents: Query<&ChildOf>,
) {
    let Some(entity) = focus.get() else { return };
    if keep.contains(entity) || parents.iter_ancestors(entity).any(|a| keep.contains(a)) {
        return;
    }
    focus.clear();
}

fn apply_actions(
    mut actions: ResMut<Actions>,
    mode: Res<State<Mode>>,
    mut next_mode: ResMut<NextState<Mode>>,
    mut level: ResMut<Level>,
    mut editor: ResMut<Editor>,
    mut history: ResMut<History>,
    mut restart: ResMut<Restart>,
    mut clear: ResMut<ClearDebris>,
    mut files: ResMut<LevelFiles>,
    mut clipboard: ResMut<Clipboard>,
    mut paste: ResMut<PendingPaste>,
    mut open: ResMut<OpenSections>,
    mut toolbar: ResMut<ToolbarOpen>,
    cameras: Query<&Transform, With<crate::visuals::WorldCamera>>,
) {
    for action in std::mem::take(&mut actions.0) {
        match action {
            Action::None => {}
            Action::TogglePlay => next_mode.set(match mode.get() {
                Mode::Edit => Mode::Play,
                Mode::Play => Mode::Edit,
            }),
            Action::Restart => restart.0 = true,
            Action::ClearDebris => clear.0 = true,
            Action::Undo => history.request = Some(HistoryStep::Undo),
            Action::Redo => history.request = Some(HistoryStep::Redo),
            Action::Add(kind) => {
                let center = cameras
                    .single()
                    .map(|t| t.translation.truncate())
                    .unwrap_or_default();
                level
                    .elements
                    .push(editor::new_element(kind, center.round()));
                editor.selected = Some(level.elements.len() - 1);
            }
            Action::Duplicate => editor::duplicate(&mut level, &mut editor),
            Action::Delete => editor::delete(&mut level, &mut editor),
            Action::Select(index) => editor.selected = Some(index),
            Action::Save => {
                files.status = match level::save(&level) {
                    Ok(place) => format!("Saved to {place}"),
                    Err(e) => format!("Save failed: {e}"),
                };
                files.names = level::list();
            }
            Action::Load(name) => match level::load(&name) {
                Ok(loaded) => {
                    *level = loaded;
                    editor.selected = None;
                    files.status = format!("Loaded {name}");
                }
                Err(e) => files.status = format!("Load failed: {e}"),
            },
            Action::Preset(name) => {
                *level = match name {
                    "pong" => level::preset_pong(),
                    "empty" => level::preset_empty(),
                    _ => level::preset_lab(),
                };
                editor.selected = None;
                files.status = format!("{name} preset (unsaved)");
            }
            Action::CopyLevel => {
                files.status = match level::to_ron(&level).map(|text| clipboard.set_text(text)) {
                    Ok(Ok(())) => "Level copied to the clipboard".into(),
                    Ok(Err(e)) => format!("Copy failed: {e:?}"),
                    Err(e) => format!("Copy failed: {e}"),
                };
            }
            Action::PasteLevel => paste.0 = Some(clipboard.fetch_text()),
            Action::ToggleSection(section) => {
                if !open.0.remove(&section) {
                    open.0.insert(section);
                }
            }
            Action::ToggleToolbar => toolbar.0 = !toolbar.0,
        }
    }
}

fn poll_paste(
    mut paste: ResMut<PendingPaste>,
    mut level: ResMut<Level>,
    mut editor: ResMut<Editor>,
    mut files: ResMut<LevelFiles>,
) {
    let Some(read) = &mut paste.0 else { return };
    let Some(result) = read.poll_result() else {
        return;
    };
    paste.0 = None;
    files.status = match result
        .map_err(|e| format!("{e:?}"))
        .and_then(|text| level::from_ron(&text))
    {
        Ok(pasted) => {
            let status = format!("Pasted {}", pasted.name);
            *level = pasted;
            editor.selected = None;
            status
        }
        Err(e) => format!("Paste failed: {e}"),
    };
}

fn show_sections(
    open: Res<OpenSections>,
    mut bodies: Query<(&SectionBody, &mut Node), Without<Chevron>>,
    mut chevrons: Query<(&Chevron, &mut Node), Without<SectionBody>>,
) {
    let display = |shown: bool| if shown { Display::Flex } else { Display::None };
    for (body, mut node) in &mut bodies {
        let shown = display(open.0.contains(&body.0));
        if node.display != shown {
            node.display = shown;
        }
    }
    for (chevron, mut node) in &mut chevrons {
        let shown = display(open.0.contains(&chevron.section) == chevron.open);
        if node.display != shown {
            node.display = shown;
        }
    }
}

fn show_for_mode(
    mode: Res<State<Mode>>,
    toolbar: Res<ToolbarOpen>,
    mut nodes: Query<(&OnlyIn, &mut Node), Without<inspector::ToolbarBody>>,
    mut toolbar_body: Query<&mut Node, With<inspector::ToolbarBody>>,
) {
    for (only, mut node) in &mut nodes {
        let shown = if only.0 == *mode.get() {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != shown {
            node.display = shown;
        }
    }
    for mut node in &mut toolbar_body {
        let shown = if toolbar.0 {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != shown {
            node.display = shown;
        }
    }
}

fn update_texts(
    mut texts: Query<(&Dyn, &mut Text)>,
    mode: Res<State<Mode>>,
    files: Res<LevelFiles>,
    level: Res<Level>,
    score: Res<Score>,
    highscores: Res<Highscores>,
    stats: Res<Stats>,
    toolbar: Res<ToolbarOpen>,
    real: Res<Time<Real>>,
    bodies: Query<(), With<RigidBody>>,
    mut fps: Local<f32>,
) {
    let dt = real.delta_secs().max(1e-6);
    *fps = if *fps == 0.0 {
        1.0 / dt
    } else {
        *fps * 0.95 + 0.05 / dt
    };
    let scoring = level.elements.iter().any(|e| e.points != 0);
    let two_players = level
        .elements
        .iter()
        .any(|e| e.owner == Some(Player::Two) || e.credit == Credit::Player(Player::Two));
    for (kind, mut text) in &mut texts {
        let value = match kind {
            Dyn::Stats => {
                let mut s = format!("{:.0} fps | {} bodies", *fps, bodies.iter().count());
                if *mode.get() == Mode::Play {
                    s += &format!(" | {} bonds | {} broken", stats.bonds, stats.broken);
                }
                s
            }
            Dyn::Status => files.status.clone(),
            Dyn::PlayToggle => match mode.get() {
                Mode::Edit => "Play (Tab)".into(),
                Mode::Play => "Edit (Tab)".into(),
            },
            Dyn::Score if !scoring => String::new(),
            Dyn::Score if two_players => {
                format!("P1 {}  :  {} P2", score.points[0], score.points[1])
            }
            Dyn::Score => format!("{}", score.points[0]),
            Dyn::Highscore if !scoring => String::new(),
            Dyn::Highscore => format!(
                "highscore {}",
                highscores.0.get(&level.name).copied().unwrap_or(0)
            ),
            Dyn::ToolbarToggle => if toolbar.0 { "»" } else { "«" }.into(),
        };
        if text.0 != value {
            text.0 = value;
        }
    }
}

/// Greys out Undo/Redo when there is nothing to undo or redo.
fn enable_history_buttons(
    mut commands: Commands,
    history: Res<History>,
    buttons: Query<(Entity, &Act, Has<InteractionDisabled>)>,
) {
    for (entity, act, disabled) in &buttons {
        let enabled = match act.0 {
            Action::Undo => history.can_undo(),
            Action::Redo => history.can_redo(),
            _ => continue,
        };
        if enabled == disabled {
            if enabled {
                commands.entity(entity).remove::<InteractionDisabled>();
            } else {
                commands.entity(entity).insert(InteractionDisabled);
            }
        }
    }
}

fn spawn_hud(mut commands: Commands) {
    commands.spawn_scene(bsn! {
        Node {
            position_type: PositionType::Absolute,
            top: px(14),
            left: px(0),
            right: px(0),
            display: Display::Flex,
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
        }
        Pickable::IGNORE
        OnlyIn(Mode::Play)
        Children [
            (
                Text("")
                TextFont { font_size: FontSize::Px(34.0) }
                TextColor(Color::WHITE)
                Pickable::IGNORE
                Dyn::Score
            ),
            (
                Text("")
                TextFont { font_size: FontSize::Px(14.0) }
                TextColor(Color::srgba(1.0, 1.0, 1.0, 0.6))
                Pickable::IGNORE
                Dyn::Highscore
            )
        ]
    });
}

/// Button variant for a selectable list row.
fn row_variant(selected: bool) -> ButtonVariant {
    if selected {
        ButtonVariant::Primary
    } else {
        ButtonVariant::Plain
    }
}
