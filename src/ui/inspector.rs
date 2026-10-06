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
    Item, button, caption, checkbox, choice, column, number, row, slider, text_input,
};
use super::{Act, Action, Dyn, OnlyIn};
use crate::editor::Editor;
use crate::level::{Body, Control, Element, Level};
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
    if element.control != Control::None {
        items.extend([
            choice("Axis", f(E::Axis)),
            slider("Max speed", f(E::Speed), 50.0, 3000.0, 0),
            slider("Range (0 = any)", f(E::Range), 0.0, 2000.0, 0),
        ]);
        if !wall {
            items.push(caption(
                "Hangs from an invisible carrier by its pins (center if none).",
            ));
        }
    }

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
            caption("Losing all pins or leaving the bounds also counts as destroyed."),
            checkbox("Respawn when destroyed", f(E::Respawn)),
            slider("Keep speed", f(E::KeepSpeed), 0.0, 3000.0, 0),
        ]);
        if element.keep_speed > 0.0 {
            items.push(slider("Speed ramp /s", f(E::SpeedRamp), 0.0, 200.0, 0));
        }
        items.push(checkbox("Bounce override", f(E::BounceOverride)));
        if element.bounce.is_some() {
            items.push(slider("Bounce", f(E::Bounce), 0.0, 1.0, 2));
        }
        items.push(checkbox("Friction override", f(E::FrictionOverride)));
        if element.friction.is_some() {
            items.push(slider("Friction", f(E::FrictionValue), 0.0, 1.5, 2));
        }
    }

    items.push(row(vec![
        button("Duplicate", Action::Duplicate, ButtonVariant::Normal),
        button("Delete", Action::Delete, ButtonVariant::Normal),
    ]));
    items
}

/// What decides which rows the inspector has; values are synced separately.
#[derive(PartialEq, Clone)]
pub struct Layout {
    selected: Option<usize>,
    kind: &'static str,
    controlled: bool,
    keep_speed: bool,
    bounce: bool,
    friction: bool,
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
        controlled: element.is_some_and(|e| e.control != Control::None),
        keep_speed: element.is_some_and(|e| e.keep_speed > 0.0),
        bounce: element.is_some_and(|e| e.bounce.is_some()),
        friction: element.is_some_and(|e| e.friction.is_some()),
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
