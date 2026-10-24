// SPDX-License-Identifier: GPL-3.0-only

//! The island's settings page, shown inside the expanded island.

use cosmic::app::Task;
use cosmic::iced::{Alignment, Length};
use cosmic::widget::{self, settings};
use cosmic::Element;

use super::Island;
use crate::config::{HomeCard, Outputs, Section};
use crate::fl;

#[derive(Debug, Clone)]
pub enum Message {
    OutputsMode(usize),
    NamedOutput(String, bool),
    HideInFullscreen(bool),
    SolidBackground(bool),
    Weather(bool),
    WeatherCompact(bool),
    WeatherQuery(String),
    WeatherSearch,
    ClockFormat(String),
    HoverDelay(u64),
    CollapseDelay(u64),
    ShowCover(bool),
    ShowTitle(bool),
    ShowBars(bool),
    ReduceMotion(bool),
    Lyrics(bool),
    ShowSection(Section, bool),
    ShowCard(HomeCard, bool),
    GoogleTasks(bool),
    GoogleCalendar(bool),
    ClipboardHistory(bool),
    OpenOnlineAccounts,
}

const MODE_FOLLOW_PANEL: usize = 0;
const MODE_ALL: usize = 1;
const MODE_ACTIVE: usize = 2;
const MODE_NAMED: usize = 3;

fn mode_of(outputs: &Outputs) -> usize {
    match outputs {
        Outputs::FollowPanel => MODE_FOLLOW_PANEL,
        Outputs::All => MODE_ALL,
        Outputs::Active => MODE_ACTIVE,
        Outputs::Named(_) => MODE_NAMED,
    }
}

/// Height of the scrolling settings page inside the island.
const PAGE_HEIGHT: f32 = 360.0;

impl Island {
    pub(super) fn update_settings(&mut self, message: Message) -> Task<super::Message> {
        match message {
            Message::OutputsMode(mode) => {
                let outputs = match mode {
                    MODE_ALL => Outputs::All,
                    MODE_ACTIVE => Outputs::Active,
                    MODE_NAMED => Outputs::Named(
                        self.outputs.iter().filter_map(|o| o.name.clone()).collect(),
                    ),
                    _ => Outputs::FollowPanel,
                };
                self.write(|c, h| c.set_outputs(h, outputs));
                return self.reconcile_surfaces();
            }
            Message::NamedOutput(name, enabled) => {
                let mut names = match &self.config.outputs {
                    Outputs::Named(names) => names.clone(),
                    _ => Vec::new(),
                };
                names.retain(|n| n != &name);
                if enabled {
                    names.push(name);
                }
                self.write(|c, h| c.set_outputs(h, Outputs::Named(names)));
                return self.reconcile_surfaces();
            }
            Message::HideInFullscreen(v) => {
                self.write(|c, h| c.set_hide_in_fullscreen(h, v));
                return self.recreate_surfaces();
            }
            Message::SolidBackground(v) => {
                self.write(|c, h| c.set_solid_background(h, v));
                // Blur again, or no more.
                for state in self.surfaces.values_mut() {
                    state.synced = None;
                }
            }
            Message::Weather(v) => {
                self.write(|c, h| c.set_weather(h, v));
                return self.fetch_weather();
            }
            Message::WeatherCompact(v) => self.write(|c, h| c.set_weather_compact(h, v)),
            Message::WeatherQuery(text) => self.weather_query = text,
            Message::WeatherSearch => {
                let query = self.weather_query.trim().to_owned();
                if query.is_empty() {
                    return Task::none();
                }
                let language = format!("{:?}", self.locale).split('_').next().unwrap_or("en").to_owned();
                return cosmic::task::future(async move {
                    super::Message::WeatherFound(crate::modules::weather::find_place(query, language).await)
                });
            }
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
            Message::ShowCard(card, show) => {
                let mut hidden = self.config.hidden_cards.clone();
                hidden.retain(|c| *c != card);
                if !show {
                    hidden.push(card);
                }
                self.write(|c, h| c.set_hidden_cards(h, hidden));
            }
            Message::GoogleTasks(v) => {
                self.write(|c, h| c.set_google_tasks(h, v));
                self.google.error = None;
                return self.sync_tasks();
            }
            Message::ClipboardHistory(v) => {
                self.write(|c, h| c.set_clipboard_history(h, v));
                self.clipboard_error = None;
                if v {
                    self.shelves.enable_history(fl!("clipboard"));
                } else {
                    self.shelves.disable_history();
                }
                self.save_shelf();
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

        // Radio buttons rather than a dropdown: a dropdown's menu would hang
        // outside the island, where clicks fall through to the desktop.
        let selected = Some(mode_of(&config.outputs));
        let mut modes = widget::Column::new().spacing(spacing.space_xxs);
        for (mode, label) in [
            (MODE_FOLLOW_PANEL, fl!("outputs-follow-panel")),
            (MODE_ALL, fl!("outputs-all")),
            (MODE_ACTIVE, fl!("outputs-active")),
            (MODE_NAMED, fl!("outputs-named")),
        ] {
            modes = modes.push(widget::radio(
                widget::text::body(label),
                mode,
                selected,
                Message::OutputsMode,
            ));
        }
        let mut displays = settings::section()
            .title(fl!("displays"))
            .add(settings::item(fl!("show-on"), modes));
        if let Outputs::Named(names) = &config.outputs {
            for name in self.outputs.iter().filter_map(|o| o.name.clone()) {
                let enabled = names.contains(&name);
                displays = displays.add(settings::item(
                    name.clone(),
                    widget::toggler(enabled)
                        .on_toggle(move |v| Message::NamedOutput(name.clone(), v)),
                ));
            }
        }

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
            .add(settings::item(
                fl!("hide-in-fullscreen"),
                widget::toggler(config.hide_in_fullscreen).on_toggle(Message::HideInFullscreen),
            ))
            .add(
                widget::Column::new()
                    .push(settings::item(
                        fl!("solid-background"),
                        widget::toggler(config.solid_background).on_toggle(Message::SolidBackground),
                    ))
                    .push(widget::text::caption(fl!("solid-background-hint")))
                    .spacing(spacing.space_xxxs),
            )
            .add(
                widget::Column::new()
                    .push(settings::item(
                        fl!("reduce-motion"),
                        widget::toggler(config.reduce_motion).on_toggle(Message::ReduceMotion),
                    ))
                    .push(widget::text::caption(fl!("reduce-motion-hint")))
                    .spacing(spacing.space_xxxs),
            );

        let place = match (&self.weather_status, &config.weather_place) {
            (Some(status), _) => status.clone(),
            (None, Some(place)) if !place.country.is_empty() => format!("{}, {}", place.name, place.country),
            (None, Some(place)) => place.name.clone(),
            (None, None) => fl!("weather-hint"),
        };
        let weather = settings::section()
            .title(fl!("weather"))
            .add(settings::item(
                fl!("weather-show"),
                widget::toggler(config.weather).on_toggle(Message::Weather),
            ))
            .add(
                widget::Column::new()
                    .push(settings::item(
                        fl!("weather-place"),
                        widget::text_input(fl!("weather-place-placeholder"), self.weather_query.clone())
                            .on_input(Message::WeatherQuery)
                            .on_submit(|_| Message::WeatherSearch)
                            .width(Length::Fixed(180.0)),
                    ))
                    .push(widget::text::caption(place))
                    .spacing(spacing.space_xxxs),
            )
            .add(settings::item(
                fl!("weather-compact"),
                widget::toggler(config.weather_compact).on_toggle(Message::WeatherCompact),
            ));

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

        let mut cards = settings::section().title(fl!("home-cards"));
        for card in HomeCard::ALL {
            // No card for a graphics card the kernel doesn't report on.
            if card == HomeCard::Gpu && !crate::modules::system::gpu_present() {
                continue;
            }
            let label = match card {
                HomeCard::Greeting => fl!("card-greeting"),
                HomeCard::Cpu => fl!("card-cpu"),
                HomeCard::Gpu => fl!("card-gpu"),
                HomeCard::Network => fl!("card-network"),
                HomeCard::Player => fl!("card-player"),
                HomeCard::Storage => fl!("card-storage"),
            };
            cards = cards.add(settings::item(
                label,
                widget::toggler(config.shows_card(card))
                    .on_toggle(move |show| Message::ShowCard(card, show)),
            ));
        }

        let history = settings::section().title(fl!("clipboard")).add(
            widget::Column::new()
                .push(settings::item(
                    fl!("clipboard-history"),
                    widget::toggler(config.clipboard_history).on_toggle(Message::ClipboardHistory),
                ))
                .push(widget::text::caption(match &self.clipboard_error {
                    Some(_) => fl!("clipboard-unavailable"),
                    None => fl!("clipboard-hint"),
                }))
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
                .push(displays)
                .push(clock)
                .push(behavior)
                .push(compact)
                .push(cards)
                .push(weather)
                .push(history)
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
