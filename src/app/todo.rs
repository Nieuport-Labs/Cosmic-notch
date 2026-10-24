// SPDX-License-Identifier: GPL-3.0-only

//! The tasks tab: a to-do list kept across restarts.

use std::time::{Duration, Instant};

use cosmic::iced::advanced::text::Wrapping;
use cosmic::iced::{Alignment, Length};
use cosmic::widget::{self, button, container};
use cosmic::Element;

use super::view::{island_button, remove_button_class, secondary_text};
use super::{Island, Message};
use crate::fl;
use crate::icons;
use crate::island::fade::{fade, faded};

/// Ticking a task off: the box pops, a line strikes the text through, then
/// the task slides away and the list closes up. The task is removed after it.
pub const TICK_ANIMATION: Duration = Duration::from_millis(950);
const POP: f32 = 0.25;
const STRIKE: f32 = 0.3;
const LEAVE_AFTER: f32 = 0.45;
const LEAVE: f32 = 0.45;
/// How far a leaving task slides to the right.
const SLIDE: f32 = 48.0;

/// Height of one task in the list.
const ROW_HEIGHT: f32 = 36.0;
/// The list scrolls beyond this height.
const LIST_HEIGHT: f32 = 300.0;

pub fn new_task_input_id() -> widget::Id {
    widget::Id::new("task-new")
}

pub fn edit_task_input_id() -> widget::Id {
    widget::Id::new("task-edit")
}

impl Island {
    pub(super) fn tasks_tab(&self) -> Element<'_, Message> {
        let spacing = cosmic::theme::spacing();

        let input = widget::Row::new()
            .push(
                widget::text_input(fl!("task-new"), self.todo_input.clone())
                    .id(new_task_input_id())
                    .on_input(Message::TodoInput)
                    .on_submit(|_| Message::TodoAdd)
                    .width(Length::Fill),
            )
            .push(island_button(
                icons::named("list-add-symbolic"),
                false,
                Message::TodoAdd,
            ))
            .spacing(spacing.space_xxs)
            .align_y(Alignment::Center);

        if self.todos.items.is_empty() {
            return widget::Column::new()
                .push(input)
                .push(self.placeholder(fl!("tasks-empty")))
                .spacing(spacing.space_xs)
                .into();
        }

        let now = Instant::now();
        let mut list = widget::Column::new().spacing(0);
        let mut height = 0.0;
        for todo in &self.todos.items {
            let id = todo.id;
            // How far the ticking-off has got: the strike-through, then leaving.
            let ticked = self.ticked.get(&id).map(|at| now.duration_since(*at).as_secs_f32());
            let strike = ticked.map_or(0.0, |t| ease_out((t / STRIKE).clamp(0.0, 1.0)));
            let leave = ticked.map_or(0.0, |t| smooth(((t - LEAVE_AFTER) / LEAVE).clamp(0.0, 1.0)));

            let text: Element<'_, Message> = match &self.todo_editing {
                Some((editing, text)) if *editing == id => widget::text_input("", text.clone())
                    .id(edit_task_input_id())
                    .on_input(Message::TodoEditInput)
                    .on_submit(|_| Message::TodoEditCommit)
                    .width(Length::Fill)
                    .into(),
                _ => {
                    let mut label = widget::text::body(todo.text.clone())
                        .wrapping(Wrapping::None)
                        .width(Length::Fill);
                    if todo.done {
                        label = label.class(cosmic::theme::Text::Custom(secondary_text));
                    }
                    let label: Element<'_, Message> = if strike > 0.0 {
                        // A line drawn across the text, about as long as the text.
                        let length = text_width(&todo.text) * strike;
                        let line = container(widget::Space::new())
                            .width(Length::Fixed(length))
                            .height(Length::Fixed(2.0))
                            .class(cosmic::theme::Container::custom(|theme| container::Style {
                                background: Some(faded(theme.cosmic().accent_color()).into()),
                                border: cosmic::iced::Border {
                                    radius: 1.0.into(),
                                    ..Default::default()
                                },
                                ..Default::default()
                            }));
                        cosmic::iced::widget::stack![
                            label,
                            container(line).height(Length::Fill).align_y(Alignment::Center)
                        ]
                        .into()
                    } else {
                        label.into()
                    };
                    widget::mouse_area(label)
                        .on_double_click(Message::TodoEditStart(id))
                        .into()
                }
            };
            let remove = button::custom(widget::icon(icons::named("window-close-symbolic")).size(12))
                .padding(4)
                .class(remove_button_class())
                .on_press(Message::TodoRemove(id));
            // The box pops when ticked.
            let pop = ticked.map_or(1.0, |t| {
                1.0 + 0.3 * (std::f32::consts::PI * (t / POP).clamp(0.0, 1.0)).sin()
            });
            let checkbox = fade(
                widget::checkbox(todo.done).on_toggle(move |done| Message::TodoToggle(id, done)),
                1.0,
                pop,
            );
            let row_height = ROW_HEIGHT * (1.0 - leave);
            height += row_height;
            let row = widget::Row::new()
                .push(checkbox)
                .push(text)
                .push(remove)
                .spacing(spacing.space_xs)
                .align_y(Alignment::Center)
                .padding([0, 0, 0, (SLIDE * leave) as u16])
                .height(Length::Fixed(ROW_HEIGHT));
            list = list.push(
                container(fade(row, 1.0 - leave, 1.0))
                    .height(Length::Fixed(row_height))
                    .align_y(Alignment::Center)
                    .clip(true),
            );
        }
        let height = height.min(LIST_HEIGHT);
        let list = widget::scrollable(container(list).padding([0, spacing.space_xs, 0, spacing.space_xxs]))
            .height(Length::Fixed(height));

        let mut status = fl!("tasks-open", count = self.todos.open_count());
        if self.config.google_tasks {
            status = format!("{status} · {}", self.google_status());
        }
        let footer = widget::text::caption(status)
            .class(cosmic::theme::Text::Custom(secondary_text))
            .wrapping(Wrapping::None);

        widget::Column::new()
            .push(input)
            .push(list)
            .push(container(footer).padding([0, spacing.space_xxs]))
            .spacing(spacing.space_xs)
            .into()
    }
}

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Rough width of body text: the strike-through needs no exact measure.
fn text_width(text: &str) -> f32 {
    text.chars().count() as f32 * 7.4 + 2.0
}
