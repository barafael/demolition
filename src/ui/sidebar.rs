//! The left sidebar: mode toggle, the World entry (level, files, physics), tools, element list,
//! play controls, materials and help. Sections open and close like an accordion.

use bevy::feathers::controls::ButtonVariant;
use bevy::feathers::theme::{ThemeBackgroundColor, ThemedText};
use bevy::feathers::tokens;
use bevy::prelude::*;
use bevy::ui_widgets::ScrollArea;

use super::bind::{Field, MatParam};
use super::widgets::{
    Item, button, caption, checkbox, choice, column, number, row, section, show_when, slider,
    text_input,
};
use super::{Action, Dyn, LevelFiles, OnlyIn, Section, row_variant};
use crate::editor::Editor;
use crate::level::Level;
use crate::materials::MaterialKind;
use crate::play::Mode;

/// Container of the element list rows.
#[derive(Component, Clone, Copy, Default)]
pub struct ElementList;

/// Container of the saved-level buttons.
#[derive(Component, Clone, Copy, Default)]
pub struct SavedLevels;

pub const WIDTH: f32 = 290.0;

fn only_in(mode: Mode, items: Vec<Item>) -> Item {
    Box::new(bsn! {
        Node {
            display: Display::Flex,
            flex_direction: FlexDirection::Column,
            row_gap: px(8),
        }
        OnlyIn(mode)
        Children [ {items} ]
    })
}

fn world_section() -> Item {
    section(
        "World",
        Section::World,
        vec![
            caption("Level name"),
            text_input(Field::LevelName),
            row(vec![
                button("Save", Action::Save, ButtonVariant::Primary),
                button("Copy link", Action::CopyLink, ButtonVariant::Normal),
            ]),
            row(vec![
                button("Copy text", Action::CopyLevel, ButtonVariant::Normal),
                button("Paste", Action::PasteLevel, ButtonVariant::Normal),
            ]),
            Box::new(
                bsn! { Text("") ThemedText Dyn::Status TextFont { font_size: FontSize::Px(12.0) } },
            ),
            caption("Presets"),
            row(vec![
                button("Lab", Action::Preset("lab"), ButtonVariant::Normal),
                button("Pong", Action::Preset("pong"), ButtonVariant::Normal),
                button("Empty", Action::Preset("empty"), ButtonVariant::Normal),
            ]),
            row(vec![
                button("Tower", Action::Preset("tower"), ButtonVariant::Normal),
                button("Wreck", Action::Preset("wreck"), ButtonVariant::Normal),
            ]),
            row(vec![
                button("Domino", Action::Preset("domino"), ButtonVariant::Normal),
                button("Pyramid", Action::Preset("pyramid"), ButtonVariant::Normal),
            ]),
            caption("Saved levels"),
            Box::new(bsn! {
                Node {
                    display: Display::Flex,
                    flex_direction: FlexDirection::Column,
                    row_gap: px(2),
                }
                SavedLevels
            }),
            slider("Gravity", Field::Gravity, -3000.0, 3000.0, 0),
            slider("Substeps (× resolution)", Field::Substeps, 1.0, 60.0, 0),
            slider(
                "Physics resolution (cells per cell)",
                Field::Resolution,
                0.25,
                4.0,
                2,
            ),
            caption(
                "Finer cells break and show stress more locally; 2 = four times the cells. Substeps scale with it so structures hold together as well as at 1, so 2 costs about 8× as much. Applied on release, so the world rebuilds.",
            ),
            Box::new(
                bsn! { Text("") ThemedText Dyn::PhysicsCost TextFont { font_size: FontSize::Px(12.0) } },
            ),
            number("View width", Field::ViewWidth),
            number("View height", Field::ViewHeight),
            number("Bounds ± x", Field::BoundsWidth),
            number("Bounds ± y", Field::BoundsHeight),
            checkbox("Gun (drag it in the editor)", Field::GunEnabled),
        ],
    )
}

fn tools() -> Item {
    column(vec![
        slider("Grid size (0 = free placement)", Field::Snap, 0.0, 100.0, 0),
        checkbox("Show grid", Field::ShowGrid),
        caption("Add"),
        row(vec![
            button("Lattice", Action::Add("Lattice"), ButtonVariant::Normal),
            button("Ball", Action::Add("Ball"), ButtonVariant::Normal),
            button("Wall", Action::Add("Wall"), ButtonVariant::Normal),
        ]),
        row(vec![
            button("Undo", Action::Undo, ButtonVariant::Normal),
            button("Redo", Action::Redo, ButtonVariant::Normal),
        ]),
        row(vec![
            button("Duplicate", Action::Duplicate, ButtonVariant::Normal),
            button("Delete", Action::Delete, ButtonVariant::Normal),
        ]),
    ])
}

fn material_params(kind: Option<MaterialKind>) -> Vec<Item> {
    let f = |p| Field::Material(kind, p);
    let mut items = vec![];
    if kind.is_some() {
        items.extend([
            slider("Density", f(MatParam::Density), 0.001, 0.2, 3),
            slider("Friction", f(MatParam::Friction), 0.0, 1.5, 2),
            slider("Restitution", f(MatParam::Restitution), 0.0, 1.0, 2),
            slider(
                "Explosive power (0 = inert)",
                f(MatParam::Explosive),
                0.0,
                5.0,
                2,
            ),
        ]);
    }
    items.extend([
        slider(
            "Point compliance (log10)",
            f(MatParam::PointCompliance),
            -9.0,
            -2.0,
            1,
        ),
        slider(
            "Angle compliance (log10)",
            f(MatParam::AngleCompliance),
            -9.0,
            -2.0,
            1,
        ),
        slider("Yield strain", f(MatParam::YieldStrain), 0.0, 2.0, 3),
        slider("Yield angle", f(MatParam::YieldAngle), 0.0, 3.0, 3),
        slider("Ductility", f(MatParam::Ductility), 0.0, 30.0, 2),
        slider("Break strain", f(MatParam::BreakStrain), 0.0, 3.0, 3),
        slider("Break angle", f(MatParam::BreakAngle), 0.0, 3.2, 3),
    ]);
    items
}

fn materials_section() -> Item {
    let mut body: Vec<Item> = vec![caption(
        "Saved with the level and applied live. Strains in cell sizes, angles in radians.",
    )];
    for kind in MaterialKind::ALL {
        body.push(section(
            kind.name(),
            Section::Material(Some(kind)),
            material_params(Some(kind)),
        ));
    }
    body.push(section(
        "Pins (bolts)",
        Section::Material(None),
        material_params(None),
    ));
    body.push(button(
        "Reset materials",
        Action::ResetMaterials,
        ButtonVariant::Normal,
    ));
    section("Materials", Section::Materials, body)
}

fn help_section() -> Item {
    section(
        "Help",
        Section::Help,
        vec![
            caption("Editor: drag to move, right/middle drag to pan, wheel to zoom."),
            caption("Dragging and arrow keys snap to the grid (Grid size above)."),
            caption("Q/E rotate (Shift: fine), arrows nudge, Ctrl+D duplicate, Del delete."),
            caption("Ctrl+Z undo, Ctrl+Shift+Z or Ctrl+Y redo. Tab plays."),
            caption("Play: R restart, C clear debris, B stress overlay, T trajectory."),
            caption("G flips gravity, M slow motion, Space pause, Tab back to the editor."),
            caption("Gun: left click fires, right click moves it, wheel sets speed, 1-6 ammo."),
            caption("Grab: right-drag picks up any element - throw it by letting go mid-swing."),
            caption("A hard yank tears pinned pieces loose. Left-drag grabs when no gun."),
        ],
    )
}

pub fn spawn(mut commands: Commands) {
    let content: Vec<Item> = vec![
        world_section(),
        only_in(
            Mode::Edit,
            vec![
                tools(),
                section(
                    "Elements",
                    Section::Elements,
                    vec![Box::new(bsn! {
                        Node {
                            display: Display::Flex,
                            flex_direction: FlexDirection::Column,
                            row_gap: px(2),
                        }
                        ElementList
                    })],
                ),
            ],
        ),
        only_in(
            Mode::Play,
            vec![
                row(vec![
                    button("Restart (R)", Action::Restart, ButtonVariant::Normal),
                    button(
                        "Clear debris (C)",
                        Action::ClearDebris,
                        ButtonVariant::Normal,
                    ),
                ]),
                // Only for levels that have a gun.
                show_when(
                    Field::GunEnabled,
                    vec![section(
                        "Gun",
                        Section::Gun,
                        vec![
                            choice("Ammo", Field::Ammo),
                            slider("Muzzle speed px/s", Field::GunSpeed, 50.0, 6000.0, 0),
                            slider(
                                "Auto-fire per second (0 = off)",
                                Field::AutoFire,
                                0.0,
                                30.0,
                                1,
                            ),
                        ],
                    )],
                ),
                section(
                    "View",
                    Section::View,
                    vec![
                        slider("Time scale", Field::TimeScale, 0.02, 1.0, 2),
                        checkbox("Paused", Field::Paused),
                        checkbox("Stress overlay", Field::StressOverlay),
                        checkbox("Trajectory preview", Field::Trajectory),
                        checkbox("Stress glow", Field::StressGlow),
                        checkbox("Shake & slow motion on big breaks", Field::ImpactFx),
                    ],
                ),
            ],
        ),
        materials_section(),
        help_section(),
    ];

    commands.spawn_scene(bsn! {
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            top: px(0),
            bottom: px(0),
            width: px(WIDTH),
            display: Display::Flex,
            flex_direction: FlexDirection::Column,
        }
        ThemeBackgroundColor(tokens::WINDOW_BG)
        Children [
            (
                Node {
                    display: Display::Flex,
                    flex_direction: FlexDirection::Column,
                    row_gap: px(6),
                    padding: px(10),
                }
                ThemeBackgroundColor(tokens::PANE_HEADER_BG)
                Children [
                    (
                        Node {
                            display: Display::Flex,
                            flex_direction: FlexDirection::Row,
                            align_items: AlignItems::Center,
                            column_gap: px(8),
                        }
                        Children [
                            (
                                Text("Demolition Lab")
                                ThemedText
                                TextFont { font_size: FontSize::Px(17.0) }
                                Node { flex_grow: 1.0 }
                            ),
                            (
                                @bevy::feathers::controls::FeathersButton {
                                    @caption: bsn! { Text("") ThemedText Dyn::PlayToggle },
                                    @variant: ButtonVariant::Primary,
                                }
                                super::Act(Action::TogglePlay)
                            )
                        ]
                    ),
                    (Text("") ThemedText Dyn::Stats TextFont { font_size: FontSize::Px(12.0) })
                ]
            ),
            (
                Node {
                    display: Display::Flex,
                    flex_direction: FlexDirection::Column,
                    flex_grow: 1.0,
                    row_gap: px(10),
                    padding: px(10),
                    overflow: Overflow::scroll_y(),
                }
                ScrollArea
                Children [ {content} ]
            )
        ]
    });
}

/// Rebuilds the element list and saved-level list when what they show changes.
pub fn rebuild_lists(
    mut commands: Commands,
    level: Res<Level>,
    editor: Res<Editor>,
    files: Res<LevelFiles>,
    element_list: Query<(Entity, Option<&Children>), With<ElementList>>,
    saved: Query<(Entity, Option<&Children>), With<SavedLevels>>,
    mut shown: Local<Option<(Vec<String>, Option<usize>)>>,
    mut shown_files: Local<Option<Vec<String>>>,
) {
    let changed = level.is_changed() || editor.is_changed() || files.is_changed();
    if !changed && shown.is_some() && shown_files.is_some() {
        return;
    }
    let rows: Vec<String> = level
        .elements
        .iter()
        .map(|e| {
            let name = if e.name.is_empty() {
                "unnamed"
            } else {
                &e.name
            };
            format!("{name} ({})", e.body.kind_name())
        })
        .collect();
    let key = (rows, editor.selected);
    if let Ok((list, children)) = element_list.single()
        && shown.as_ref() != Some(&key)
    {
        for child in children.into_iter().flatten() {
            commands.entity(*child).despawn();
        }
        for (index, label) in key.0.iter().enumerate() {
            let variant = row_variant(key.1 == Some(index));
            commands
                .spawn_scene(button(label.clone(), Action::Select(index), variant))
                .insert(ChildOf(list));
        }
        *shown = Some(key);
    }

    if let Ok((list, children)) = saved.single()
        && shown_files.as_ref() != Some(&files.names)
    {
        for child in children.into_iter().flatten() {
            commands.entity(*child).despawn();
        }
        if files.names.is_empty() {
            commands
                .spawn_scene(caption("none yet"))
                .insert(ChildOf(list));
        }
        for name in &files.names {
            commands
                .spawn_scene(button(
                    name.clone(),
                    Action::Load(name.clone()),
                    ButtonVariant::Plain,
                ))
                .insert(ChildOf(list));
        }
        *shown_files = Some(files.names.clone());
    }
}
