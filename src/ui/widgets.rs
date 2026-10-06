//! Small scene builders for the panels. Each returns a boxed scene so panels can be assembled
//! from plain `Vec`s.

use bevy::feathers::constants::icons;
use bevy::feathers::controls::{
    ButtonVariant, FeathersButton, FeathersCheckbox, FeathersMenu, FeathersMenuButton,
    FeathersMenuItem, FeathersMenuPopup, FeathersNumberInput, FeathersSlider, FeathersTextInput,
    FeathersTextInputContainer,
};
use bevy::feathers::display::{icon, label_small};
use bevy::feathers::theme::ThemedText;
use bevy::prelude::*;
use bevy::ui_widgets::SliderPrecision;

use super::bind::{Bind, ChoiceCaption, ChoiceItem, Field, ShowWhen, Shown};
use super::{Act, Action, Chevron, Section, SectionBody};

pub type Item = Box<dyn Scene>;

/// A vertical stack.
pub fn column(items: Vec<Item>) -> Item {
    Box::new(bsn! {
        Node {
            display: Display::Flex,
            flex_direction: FlexDirection::Column,
            row_gap: px(6),
        }
        Children [ {items} ]
    })
}

/// A horizontal row whose items share the width.
pub fn row(items: Vec<Item>) -> Item {
    Box::new(bsn! {
        Node {
            display: Display::Flex,
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: px(6),
        }
        Children [ {items} ]
    })
}

/// A vertical stack shown only while `field` is on (see `ShowWhen`).
pub fn show_when(field: Field, items: Vec<Item>) -> Item {
    Box::new(bsn! {
        Node {
            display: Display::Flex,
            flex_direction: FlexDirection::Column,
            row_gap: px(6),
        }
        ShowWhen(field)
        Children [ {items} ]
    })
}

pub fn caption(text: impl Into<String>) -> Item {
    Box::new(label_small(text))
}

pub fn button(text: impl Into<String>, action: Action, variant: ButtonVariant) -> Item {
    let text: String = text.into();
    Box::new(bsn! {
        @FeathersButton {
            @caption: bsn! { Text({text}) ThemedText },
            @variant: variant,
        }
        Node { flex_grow: 1.0 }
        Act({action})
    })
}

pub fn slider(text: impl Into<String>, field: Field, min: f32, max: f32, precision: i32) -> Item {
    let text: String = text.into();
    Box::new(bsn! {
        Node {
            display: Display::Flex,
            flex_direction: FlexDirection::Column,
            row_gap: px(2),
        }
        Children [
            label_small(text),
            (
                @FeathersSlider { @min: min, @max: max, @value: min }
                SliderPrecision(precision)
                Bind(field)
            )
        ]
    })
}

pub fn checkbox(text: impl Into<String>, field: Field) -> Item {
    let text: String = text.into();
    Box::new(bsn! {
        @FeathersCheckbox { @caption: bsn! { Text({text}) ThemedText } }
        Bind(field)
    })
}

/// A labelled number input.
pub fn number(text: impl Into<String>, field: Field) -> Item {
    let text: String = text.into();
    Box::new(bsn! {
        Node {
            display: Display::Flex,
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: px(6),
        }
        Children [
            (label_small(text) Node { width: px(96) }),
            (
                @FeathersNumberInput
                Node { flex_grow: 1.0 }
                Bind(field)
                Shown
            )
        ]
    })
}

/// A dropdown that picks one of `field.choices()`.
pub fn choice(text: impl Into<String>, field: Field) -> Item {
    let text: String = text.into();
    let items: Vec<Item> = field
        .choices()
        .into_iter()
        .enumerate()
        .map(|(index, name)| -> Item {
            Box::new(bsn! {
                @FeathersMenuItem { @caption: bsn! { Text({name}) ThemedText } }
                ChoiceItem { field: field, index: index }
            })
        })
        .collect();
    Box::new(bsn! {
        Node {
            display: Display::Flex,
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: px(6),
        }
        Children [
            (label_small(text) Node { width: px(96) }),
            (
                @FeathersMenu
                Node { flex_grow: 1.0 }
                Children [
                    (
                        @FeathersMenuButton {
                            @caption: bsn! { Text("") ThemedText ChoiceCaption Bind(field) }
                        }
                        Node { flex_grow: 1.0 }
                    ),
                    (
                        @FeathersMenuPopup
                        Children [ {items} ]
                    )
                ]
            )
        ]
    })
}

pub fn text_input(field: Field) -> Item {
    Box::new(bsn! {
        @FeathersTextInputContainer
        Node { flex_grow: 1.0 }
        Children [
            (
                @FeathersTextInput { @max_characters: 200usize }
                Bind(field)
            )
        ]
    })
}

/// An accordion entry: a header that opens and closes the body below it.
pub fn section(title: impl Into<String>, id: Section, body: Vec<Item>) -> Item {
    let title: String = title.into();
    Box::new(bsn! {
        Node {
            display: Display::Flex,
            flex_direction: FlexDirection::Column,
            row_gap: px(6),
        }
        Children [
            (
                @FeathersButton {
                    @caption: bsn_list! [
                        (icon(icons::CHEVRON_RIGHT) Chevron { section: id, open: false }),
                        (icon(icons::CHEVRON_DOWN) Chevron { section: id, open: true }),
                        (Text({title}) ThemedText Node { margin: UiRect::left(px(4)) })
                    ],
                    @variant: ButtonVariant::Plain,
                }
                Node { justify_content: JustifyContent::Start }
                Act({Action::ToggleSection(id)})
            ),
            (
                Node {
                    display: Display::Flex,
                    flex_direction: FlexDirection::Column,
                    row_gap: px(6),
                    padding: UiRect::left(px(10)),
                }
                SectionBody(id)
                Children [ {body} ]
            )
        ]
    })
}
