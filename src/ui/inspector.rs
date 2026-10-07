//! The collapsible toolbar on the right: tweaks for the selected element. Its contents are
//! rebuilt when the selection or the element's shape of settings changes (kind, control, ...);
//! values themselves are kept current by the bindings.

use bevy::feathers::controls::{ButtonVariant, FeathersToolButton};
use bevy::feathers::theme::{ThemeBackgroundColor, ThemedText};
use bevy::feathers::tokens;
use bevy::prelude::*;
use bevy::ui_widgets::ScrollArea;

use super::bind::{ElementField as E, Field};
use super::widgets::{
    Item, button, caption, checkbox, choice, column, number, row, show_when, show_when_choice,
    slider, text_input,
};
use super::{Act, Action, Dyn, OnlyIn};
use crate::editor::Editor;
use crate::level::{Body, Element, Level, PinStyle};
use crate::play::Mode;

/// The part of the toolbar that collapses.
#[derive(Component, Clone, Copy, Default)]
pub struct ToolbarBody;

/// Container the inspector rows are spawned into.
#[derive(Component, Clone, Copy, Default)]
pub struct InspectorBody;

pub const WIDTH: f32 = 300.0;

pub fn spawn_toolbar(mut commands: Commands) {
    commands.spawn_scene(bsn! {
        Node {
            position_type: PositionType::Absolute,
            right: px(0),
            top: px(0),
            bottom: px(0),
            display: Display::Flex,
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Start,
        }
        OnlyIn(Mode::Edit)
        Children [
            (
                @FeathersToolButton {
                    @caption: bsn! { Text("") ThemedText Dyn::ToolbarToggle },
                    @variant: ButtonVariant::Normal,
                }
                Node { margin: UiRect::all(px(6)) }
                Act(Action::ToggleToolbar)
            ),
            (
                Node {
                    width: px(WIDTH),
                    height: percent(100),
                    display: Display::Flex,
                    flex_direction: FlexDirection::Column,
                    row_gap: px(8),
                    padding: px(10),
                    overflow: Overflow::scroll_y(),
                }
                ThemeBackgroundColor(tokens::WINDOW_BG)
                ScrollArea
                ToolbarBody
                Children [
                    (Text("Object") ThemedText TextFont { font_size: FontSize::Px(17.0) }),
                    (
                        Node {
                            display: Display::Flex,
                            flex_direction: FlexDirection::Column,
                            row_gap: px(8),
                        }
                        InspectorBody
                    )
                ]
            )
        ]
    });
}

fn heading(text: &str) -> Item {
    let text = text.to_string();
    Box::new(bsn! {
        Text({text})
        ThemedText
        TextFont { font_size: FontSize::Px(14.0) }
        Node { margin: UiRect::top(px(6)) }
    })
}

fn rows(element: &Element) -> Vec<Item> {
    let f = Field::Element;
    let wall = matches!(element.body, Body::Wall { .. });
    let mut items: Vec<Item> = vec![
        caption("Name"),
        text_input(f(E::Name)),
        choice("Kind", f(E::Kind)),
        number("x", f(E::PosX)),
        number("y", f(E::PosY)),
        number("Angle °", f(E::Angle)),
    ];

    items.push(heading("Body"));
    match element.body {
        Body::Lattice { .. } => items.extend([
            choice("Material", f(E::Material)),
            number("Columns", f(E::Cols)),
            number("Rows", f(E::Rows)),
            slider("Cell size", f(E::Cell), 3.0, 40.0, 1),
            checkbox("Round", f(E::Round)),
        ]),
        Body::Ball { .. } => items.extend([
            slider("Radius", f(E::Radius), 1.0, 100.0, 1),
            slider("Density", f(E::Density), 0.001, 0.2, 3),
            slider("Restitution", f(E::Restitution), 0.0, 1.0, 2),
            slider("Friction", f(E::Friction), 0.0, 1.5, 2),
        ]),
        Body::Wall { .. } => {
            items.extend([number("Width", f(E::Width)), number("Height", f(E::Height))])
        }
    }
    if !matches!(element.body, Body::Lattice { .. }) {
        items.extend([
            slider("Red", f(E::Red), 0.0, 1.0, 2),
            slider("Green", f(E::Green), 0.0, 1.0, 2),
            slider("Blue", f(E::Blue), 0.0, 1.0, 2),
        ]);
    }

    if !wall {
        items.push(heading("Pins"));
        items.extend([
            row(vec![
                checkbox("Left", f(E::PinLeft)),
                checkbox("Right", f(E::PinRight)),
            ]),
            row(vec![
                checkbox("Top", f(E::PinTop)),
                checkbox("Bottom", f(E::PinBottom)),
            ]),
            checkbox("Center", f(E::PinCenter)),
            number("Every n-th", f(E::PinEvery)),
            choice("Pin style", f(E::PinStyle)),
            show_when_choice(
                f(E::PinStyle),
                PinStyle::ALL
                    .iter()
                    .position(|p| *p == PinStyle::Rope)
                    .unwrap_or(2),
                vec![
                    slider("Rope length", f(E::PinRope), 10.0, 1000.0, 0),
                    caption("Hangs from a point this far straight above the pin."),
                ],
            ),
            caption(
                "Pins hold on to whatever they touch (another element or a wall), else the world.",
            ),
            number("Velocity x", f(E::VelX)),
            number("Velocity y", f(E::VelY)),
        ]);
    }

    items.push(heading("Control"));
    items.push(choice("Owner", f(E::Owner)));
    items.push(caption(
        "What an owned element hits counts as hit by its owner.",
    ));
    items.push(choice("Input", f(E::Control)));
    let mut controlled = vec![
        choice("Axis", f(E::Axis)),
        slider("Max speed", f(E::Speed), 50.0, 3000.0, 0),
        slider("Range (0 = any)", f(E::Range), 0.0, 2000.0, 0),
    ];
    if !wall {
        controlled.push(caption(
            "Hangs from an invisible carrier by its pins (center if none).",
        ));
    }
    items.push(show_when(f(E::Control), controlled));

    if !wall {
        items.push(heading("Destruction"));
        items.extend([
            number("Points", f(E::Points)),
            choice("Credit to", f(E::Credit)),
        ]);
        if matches!(element.body, Body::Lattice { .. }) {
            items.push(slider(
                "Destroyed at bond loss",
                f(E::DestroyedAt),
                0.0,
                1.0,
                2,
            ));
        }
        items.extend([
            slider(
                "Destroyed when tipped (°, 0 = off)",
                f(E::TippedAt),
                0.0,
                180.0,
                0,
            ),
            caption("Losing all pins or leaving the bounds also counts as destroyed."),
            checkbox("Respawn when destroyed", f(E::Respawn)),
            slider("Keep speed", f(E::KeepSpeed), 0.0, 3000.0, 0),
        ]);
        items.extend([
            show_when(
                f(E::KeepSpeed),
                vec![slider("Speed ramp /s", f(E::SpeedRamp), 0.0, 200.0, 0)],
            ),
            checkbox("Bounce override", f(E::BounceOverride)),
            show_when(
                f(E::BounceOverride),
                vec![slider("Bounce", f(E::Bounce), 0.0, 1.0, 2)],
            ),
            checkbox("Friction override", f(E::FrictionOverride)),
            show_when(
                f(E::FrictionOverride),
                vec![slider("Friction", f(E::FrictionValue), 0.0, 1.5, 2)],
            ),
        ]);
    }

    items.push(row(vec![
        button("Duplicate", Action::Duplicate, ButtonVariant::Normal),
        button("Delete", Action::Delete, ButtonVariant::Normal),
    ]));
    items
}

/// What decides which rows the inspector has. Values are synced by the bindings, and rows that
/// depend on other values are shown and hidden with `ShowWhen`, so editing never rebuilds the
/// inspector under the user's pointer.
#[derive(PartialEq, Clone)]
pub struct Layout {
    selected: Option<usize>,
    kind: &'static str,
}

pub fn rebuild(
    mut commands: Commands,
    level: Res<Level>,
    editor: Res<Editor>,
    body: Query<(Entity, Option<&Children>), With<InspectorBody>>,
    mut shown: Local<Option<Layout>>,
) {
    let element = editor.selected.and_then(|i| level.elements.get(i));
    let layout = Layout {
        selected: element.and(editor.selected),
        kind: element.map_or("", |e| e.body.kind_name()),
    };
    let Ok((container, children)) = body.single() else {
        return;
    };
    if shown.as_ref() == Some(&layout) {
        return;
    }
    for child in children.into_iter().flatten() {
        commands.entity(*child).despawn();
    }
    let content = match element {
        Some(element) => column(rows(element)),
        None => caption("Select an element to tweak it, or add one from the sidebar."),
    };
    commands.spawn_scene(content).insert(ChildOf(container));
    *shown = Some(layout);
}
