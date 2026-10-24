// SPDX-License-Identifier: GPL-3.0-only

//! The island's settings page, shown inside the expanded island.

use cosmic::app::Task;
use cosmic::iced::{Alignment, Length};
use cosmic::widget::{self, settings};
use cosmic::Element;

use super::Island;
use crate::config::Section;
use crate::fl;

#[derive(Debug, Clone)]
pub enum Message {
    ClockFormat(String),
    HoverDelay(u64),
    CollapseDelay(u64),
    ShowCover(bool),
    ShowTitle(bool),
    ShowBars(bool),
    ReduceMotion(bool),
    Lyrics(bool),
    ShowSection(Section, bool),
    GoogleTasks(bool),
    GoogleCalendar(bool),
    OpenOnlineAccounts,
}

/// Height of the scrolling settings page inside the island.
const PAGE_HEIGHT: f32 = 360.0;

impl Island {
    pub(super) fn update_settings(&mut self, message: Message) -> Task<super::Message> {
        match message {
            Message::ClockFormat(format) => {
                let format = (!format.trim().is_empty()).then_some(format);
                self.write(|c, h| c.set_clock_format(h, format));
            }
            Message::HoverDelay(ms) => self.write(|c, h| c.set_hover_delay_ms(h, ms)),
            Message::CollapseDelay(ms) => self.write(|c, h| c.set_collapse_delay_ms(h, ms)),
            Message::ShowCover(v) => self.write(|c, h| c.set_show_cover(h, v)),
            Message::ShowTitle(v) => self.write(|c, h| c.set_show_title(h, v)),
            Message::ShowBars(v) => self.write(|c, h| c.set_show_bars(h, v)),
            Message::ReduceMotion(v) => self.write(|c, h| c.set_reduce_motion(h, v)),
            Message::ShowSection(section, show) => {
                let mut hidden = self.config.hidden_sections.clone();
                hidden.retain(|s| *s != section);
                if !show {
                    hidden.push(section);
                }
                // The island never goes empty: the last section stays.
                if hidden.len() < Section::ALL.len() {
                    self.write(|c, h| c.set_hidden_sections(h, hidden));
                }
            }
            Message::GoogleTasks(v) => {
                self.write(|c, h| c.set_google_tasks(h, v));
                self.google.error = None;
                return self.sync_tasks();
            }
            Message::GoogleCalendar(v) => {
                self.write(|c, h| c.set_google_calendar(h, v));
                self.calendar.error = None;
                return self.sync_calendar();
            }
            Message::OpenOnlineAccounts => {
                return cosmic::task::message(cosmic::Action::App(super::Message::OpenOnlineAccounts));
            }
            Message::Lyrics(v) => {
                self.write(|c, h| c.set_lyrics(h, v));
                return self.load_lyrics();
            }
        }
        Task::none()
    }

    /// One line on how syncing with Google goes.
    pub(super) fn google_status(&self) -> String {
        use crate::modules::google_tasks::Error;
        match (&self.google.error, &self.google.account) {
            (Some(Error::NoAccount), _) => fl!("google-no-account"),
            (Some(Error::NoPermission), _) => fl!("google-no-permission"),
            (Some(Error::Other(err)), _) => fl!("google-error", error = err.clone()),
            (None, Some(account)) if self.config.google_tasks => {
                fl!("google-account", account = account.clone())
            }
            _ => fl!("google-hint"),
        }
    }

    /// Applies a setter to the config and persists it.
    fn write(
        &mut self,
        set: impl FnOnce(
            &mut crate::config::IslandConfig,
            &cosmic::cosmic_config::Config,
        ) -> Result<bool, cosmic::cosmic_config::Error>,
    ) {
        if let Some(handle) = &self.config_handle
            && let Err(err) = set(&mut self.config, handle)
        {
            tracing::error!("failed to save settings: {err}");
        }
    }

    pub(super) fn settings_tab(&self) -> Element<'_, Message> {
        let spacing = cosmic::theme::spacing();
        let config = &self.config;

        // The last shown section can't be switched off, so the island never empties.
        let shown = Section::ALL.iter().filter(|s| config.shows(**s)).count();
        let mut sections = settings::section().title(fl!("sections"));
        for section in Section::ALL {
            let show = config.shows(section);
            let label = match section {
                Section::Home => fl!("section-home"),
                Section::Media => fl!("section-media"),
                Section::Shelf => fl!("section-shelf"),
                Section::Calendar => fl!("section-calendar"),
                Section::Tasks => fl!("section-tasks"),
            };
            let toggle = widget::toggler(show);
            let toggle = if show && shown == 1 {
                toggle
            } else {
                toggle.on_toggle(move |v| Message::ShowSection(section, v))
            };
            sections = sections.add(settings::item(label, toggle));
        }

        let clock = settings::section().title(fl!("clock")).add(
            widget::Column::new()
                .push(settings::item(
                    fl!("clock-format"),
                    widget::text_input("", config.clock_format.clone().unwrap_or_default())
                        .on_input(Message::ClockFormat)
                        .width(Length::Fixed(180.0)),
                ))
                .push(widget::text::caption(fl!("clock-format-hint")))
                .spacing(spacing.space_xxxs),
        );

        let behavior = settings::section()
            .title(fl!("behavior"))
            .add(settings::item(
                fl!("hover-delay"),
                widget::spin_button(
                    config.hover_delay_ms.to_string(),
                    config.hover_delay_ms,
                    50,
                    0,
                    2000,
                    Message::HoverDelay,
                ),
            ))
            .add(settings::item(
                fl!("collapse-delay"),
                widget::spin_button(
                    config.collapse_delay_ms.to_string(),
                    config.collapse_delay_ms,
                    50,
                    0,
                    3000,
                    Message::CollapseDelay,
                ),
            ))
            .add(
                widget::Column::new()
                    .push(settings::item(
                        fl!("reduce-motion"),
                        widget::toggler(config.reduce_motion).on_toggle(Message::ReduceMotion),
                    ))
                    .push(widget::text::caption(fl!("reduce-motion-hint")))
                    .spacing(spacing.space_xxxs),
            );

        let compact = settings::section()
            .title(fl!("compact"))
            .add(settings::item(
                fl!("show-cover"),
                widget::toggler(config.show_cover).on_toggle(Message::ShowCover),
            ))
            .add(settings::item(
                fl!("show-title"),
                widget::toggler(config.show_title).on_toggle(Message::ShowTitle),
            ))
            .add(settings::item(
                fl!("show-bars"),
                widget::toggler(config.show_bars).on_toggle(Message::ShowBars),
            ));

        let lyrics = settings::section().title(fl!("lyrics")).add(
            widget::Column::new()
                .push(settings::item(
                    fl!("lyrics-enable"),
                    widget::toggler(config.lyrics).on_toggle(Message::Lyrics),
                ))
                .push(widget::text::caption(fl!("lyrics-privacy")))
                .spacing(spacing.space_xxxs),
        );

        let tasks = settings::section().title(fl!("google")).add(
            widget::Column::new()
                .push(settings::item(
                    fl!("google-tasks"),
                    widget::toggler(config.google_tasks).on_toggle(Message::GoogleTasks),
                ))
                .push(widget::text::caption(self.google_status()))
                .push(settings::item(
                    fl!("google-calendar"),
                    widget::toggler(config.google_calendar).on_toggle(Message::GoogleCalendar),
                ))
                .push(widget::text::caption(self.calendar_status()))
                .push(
                    widget::button::link(fl!("open-online-accounts"))
                        .padding(0)
                        .on_press(Message::OpenOnlineAccounts),
                )
                .spacing(spacing.space_xxxs),
        );

        widget::scrollable(
            widget::Column::new()
                .push(sections)
                .push(clock)
                .push(behavior)
                .push(compact)
                .push(tasks)
                .push(lyrics)
                .spacing(spacing.space_s)
                .padding([0, spacing.space_xs, 0, 0])
                .align_x(Alignment::Start),
        )
        .height(Length::Fixed(PAGE_HEIGHT))
        .into()
    }
}
