// SPDX-License-Identifier: GPL-3.0-only

//! What the island draws. Colors, radii, spacing and fonts all come from the
//! user's COSMIC theme; nothing here is a fixed color. Every color of the
//! island's content goes through [`faded`], so content can fade in.

use std::time::{Duration, Instant};

use cosmic::iced::advanced::text::{Ellipsize, EllipsizeHeightLimit, Wrapping};
use cosmic::iced::clipboard::dnd::DndAction;
use cosmic::iced::{Alignment, Color, Length, Size, window};
use cosmic::widget::{self, button, container, icon};
use cosmic::Element;

use super::{Island, Message, Tab};
use crate::fl;
use crate::icons;
use crate::island::fade::{fade, faded};
use crate::island::geometry;
use crate::island::measure::measure;
use crate::modules::clock;
use crate::modules::lyrics;
use crate::modules::media::{self, MediaState, Status};
use crate::modules::shelf::{Clip, DragOut, Item};

/// Width of the expanded island's content, from the design.
pub const EXPANDED_WIDTH: f32 = 640.0;
/// The lyrics page: size of the cover, and of the lines of the song.
const LYRICS_COVER: f32 = 130.0;
const LYRICS_LINE: f32 = 40.0;
const LYRICS_ROWS: usize = 3;
const LYRICS_HEIGHT: f32 = LYRICS_LINE * LYRICS_ROWS as f32;
const LYRICS_SMALL: f32 = 14.0;
const LYRICS_LARGE: f32 = 25.0;
/// How far a new tab slides in from.
const TAB_SLIDE: f32 = 56.0;
/// Size of a file's preview on the shelf; its name is no wider.
const PREVIEW: f32 = 88.0;

/// Corner radii of the island, taken from the theme.
#[derive(Debug, Clone, Copy)]
pub struct Radii {
    compact: f32,
    expanded: f32,
}

impl Radii {
    pub fn from_theme(theme: &cosmic::cosmic_theme::Theme) -> Self {
        Self {
            compact: theme.radius_xl()[0],
            expanded: theme.radius_m()[0],
        }
    }

    pub fn pill(&self, expanded: bool) -> f32 {
        if expanded { self.expanded } else { self.compact }
    }
}

impl Island {
    pub(super) fn island_view(&self, id: window::Id) -> Element<'_, Message> {
        let Some(state) = self.surfaces.get(&id) else {
            return widget::Space::new().into();
        };
        let now = Instant::now();
        let size = state.anim.at(now);
        let expanded = state.expanded;
        let (opacity, scale) = geometry::content_transition(state.content_since, now);
        let glow = state.glow(now);

        let content = if expanded {
            self.expanded_view(id, opacity)
        } else {
            self.compact_view(self.panel.compact_height as f32, opacity)
        };
        let content = fade(content, opacity, scale);

        let solid = self.config.solid_background;
        let pill = container(measure(content, move |s| Message::Measured(id, s)))
            .width(Length::Fixed(size.width))
            .height(Length::Fixed(size.height))
            .align_x(Alignment::Center)
            .align_y(Alignment::Start)
            .clip(true)
            .class(cosmic::theme::Container::custom(move |theme| {
                pill_style(theme, size, expanded, glow, solid)
            }));

        let pill = widget::mouse_area(pill)
            .on_enter(Message::PointerEntered(id))
            .on_exit(Message::PointerLeft(id))
            .on_press(Message::PillPressed(id))
            .on_middle_press(Message::MediaCommand(media::Command::PlayPause));

        let pill = widget::dnd_destination::dnd_destination_for_data::<Clip, _>(
            pill,
            move |clip, _| Message::Dropped(id, clip),
        )
        .on_enter(move |_, _, _| Message::DragEntered(id))
        .on_leave(move || Message::DragLeft(id));

        container(pill)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Alignment::Center)
            .align_y(Alignment::Start)
            .into()
    }

    fn compact_view(&self, height: f32, opacity: f32) -> Element<'_, Message> {
        let spacing = cosmic::theme::spacing();
        let mut row = widget::Row::new()
            .align_y(Alignment::Center)
            .spacing(spacing.space_xs)
            .padding([0, spacing.space_xxs])
            .height(Length::Fixed(height));

        if let Some(media) = &self.media {
            let mut now_playing = widget::Row::new()
                .align_y(Alignment::Center)
                .spacing(spacing.space_xxxs);
            if self.config.show_cover {
                now_playing = now_playing.push(self.cover((height * 0.55).round(), true, opacity));
            }
            if self.config.show_title {
                now_playing = now_playing.push(
                    container(
                        widget::text::body(media.label())
                            .font(cosmic::font::semibold())
                            .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1))),
                    )
                    .max_width(self.config.title_max_width),
                );
            }
            if self.bars_enabled() && media.status == Status::Playing {
                now_playing = now_playing.push(self.bars((height * 0.55).round()));
            }
            row = row.push(now_playing);
        }

        if self.config.weather_compact
            && let Some(weather) = self.shown_weather()
        {
            row = row.push(
                widget::Row::new()
                    .push(
                        widget::icon(icons::named(crate::modules::weather::icon(weather.code, weather.day)))
                            .size((height * 0.5).round() as u16),
                    )
                    .push(
                        widget::text::body(crate::modules::weather::format_temperature(weather.temperature))
                            .font(cosmic::font::semibold()),
                    )
                    .spacing(spacing.space_xxxs)
                    .align_y(Alignment::Center),
            );
        }
        row.push(widget::text::body(self.clock_text()).font(cosmic::font::semibold())).into()
    }

    /// Album art, or a note icon on a plate when there is none.
    pub(super) fn cover(&self, size: f32, small: bool, opacity: f32) -> Element<'_, Message> {
        let theme = cosmic::theme::active();
        let radius = if small {
            theme.cosmic().radius_xs()
        } else {
            theme.cosmic().radius_m()
        };
        if let Some((_, Some(handle))) = &self.art {
            return widget::image(handle.clone())
                .width(Length::Fixed(size))
                .height(Length::Fixed(size))
                .content_fit(cosmic::iced::ContentFit::Cover)
                .border_radius(radius)
                .opacity(opacity)
                .into();
        }
        container(
            widget::icon(icons::named("audio-x-generic-symbolic")).size((size * 0.5) as u16),
        )
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .class(cosmic::theme::Container::custom(move |theme| plate_style(theme, radius)))
        .into()
    }

    /// Bars in the accent color, following the levels of what plays.
    pub(super) fn bars(&self, height: f32) -> Element<'static, Message> {
        let mut row = widget::Row::new().spacing(2).align_y(Alignment::Center);
        for level in self.bar_levels {
            row = row.push(
                container(widget::Space::new())
                    .width(Length::Fixed(2.0))
                    .height(Length::Fixed((height * (0.15 + 0.85 * level)).max(2.0)))
                    .class(cosmic::theme::Container::custom(|theme| container::Style {
                        background: Some(faded(theme.cosmic().accent_color()).into()),
                        border: cosmic::iced::Border {
                            radius: 1.0.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    })),
            );
        }
        container(row)
            .height(Length::Fixed(height))
            .align_y(Alignment::Center)
            .into()
    }

    fn media_tab(&self, opacity: f32) -> Element<'_, Message> {
        let Some(media) = &self.media else {
            return self.placeholder(fl!("nothing-playing"));
        };
        let spacing = cosmic::theme::spacing();

        if self.lyrics_page() {
            // The lyrics get the room: the cover and the track on the left, the
            // song on the right, running through the middle of the page.
            let side = widget::Column::new()
                .push(self.cover(LYRICS_COVER, false, opacity))
                .push(
                    widget::text::heading(media.title.clone())
                        .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(2)))
                        .width(Length::Fixed(LYRICS_COVER)),
                )
                .push(
                    widget::text::caption(media.artists.clone())
                        .class(cosmic::theme::Text::Custom(secondary_text))
                        .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1)))
                        .width(Length::Fixed(LYRICS_COVER)),
                )
                .spacing(spacing.space_xxs);
            return widget::Column::new()
                .push(
                    widget::Row::new()
                        .push(side)
                        .push(self.lyrics_panel(media))
                        .spacing(spacing.space_m),
                )
                .push(self.progress(media))
                .push(self.media_controls(media))
                .spacing(spacing.space_s)
                .into();
        }

        // Compact: the cover with the track beside it, and one row with the
        // controls, the progress and the lyrics button.
        const COVER: f32 = 96.0;
        let mut title_row = widget::Row::new()
            .push(
                widget::text::title4(media.title.clone())
                    .font(cosmic::font::bold())
                    .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1)))
                    .width(Length::Fill),
            )
            .spacing(spacing.space_xs)
            .align_y(Alignment::Center);
        if media.status == Status::Playing && self.bars_enabled() {
            title_row = title_row.push(self.bars(18.0));
        }
        let mut artist = media.artists.clone();
        if let Some(album) = &media.album
            && !album.is_empty()
            && *album != media.title
        {
            artist = format!("{artist} · {album}");
        }
        let details = widget::Column::new()
            .push(title_row)
            .push(
                widget::text::body(artist)
                    .font(cosmic::font::semibold())
                    .class(cosmic::theme::Text::Custom(secondary_text))
                    .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1))),
            )
            .push(widget::Space::new().height(Length::Fill))
            .push(self.lyrics_snippet(media))
            .spacing(spacing.space_xxxs)
            .width(Length::Fill)
            .height(Length::Fixed(COVER));

        widget::Column::new()
            .push(
                widget::Row::new()
                    .push(self.cover(COVER, false, opacity))
                    .push(details)
                    .spacing(spacing.space_s),
            )
            .push(self.compact_controls(media))
            .spacing(spacing.space_xs)
            .into()
    }

    /// Previous, play and next, the progress, and the lyrics button, in one row.
    fn compact_controls(&self, media: &MediaState) -> Element<'_, Message> {
        let spacing = cosmic::theme::spacing();
        let button = |icon: &'static str, command: Option<media::Command>| {
            island_button_sized(icons::named(icon), false, command.map(Message::MediaCommand), 20)
        };
        let mut row = widget::Row::new()
            .push(button(
                "media-skip-backward-symbolic",
                media.can_go_previous.then_some(media::Command::Previous),
            ))
            .push(button(
                if media.status == Status::Playing {
                    "media-playback-pause-symbolic"
                } else {
                    "media-playback-start-symbolic"
                },
                Some(media::Command::PlayPause),
            ))
            .push(button(
                "media-skip-forward-symbolic",
                media.can_go_next.then_some(media::Command::Next),
            ))
            .push(container(self.progress(media)).width(Length::Fill).padding([0, spacing.space_xs]))
            .spacing(spacing.space_xxxs)
            .align_y(Alignment::Center);
        if self.config.lyrics {
            row = row.push(widget::tooltip(
                island_button(
                    icons::named("audio-input-microphone-symbolic"),
                    self.lyrics_view,
                    Message::ToggleLyrics,
                ),
                widget::text::caption(fl!("lyrics")),
                widget::tooltip::Position::Top,
            ));
        }
        row.into()
    }

    fn media_controls(&self, media: &MediaState) -> Element<'_, Message> {
        let spacing = cosmic::theme::spacing();
        // Balances the lyrics button on the right, so the controls stay centered.
        let side = f32::from(spacing.space_xxs) * 2.0 + 20.0;
        let lyrics: Element<'_, Message> = if self.config.lyrics {
            widget::tooltip(
                island_button(
                    icons::named("audio-input-microphone-symbolic"),
                    self.lyrics_view,
                    Message::ToggleLyrics,
                ),
                widget::text::caption(fl!("lyrics")),
                widget::tooltip::Position::Top,
            )
            .into()
        } else {
            widget::Space::new().width(Length::Fixed(side)).into()
        };
        widget::Row::new()
            .push(widget::Space::new().width(Length::Fixed(side)))
            .push(widget::Space::new().width(Length::Fill))
            .push(control(
                "media-skip-backward-symbolic",
                media.can_go_previous.then_some(media::Command::Previous),
            ))
            .push(control(
                if media.status == Status::Playing {
                    "media-playback-pause-symbolic"
                } else {
                    "media-playback-start-symbolic"
                },
                Some(media::Command::PlayPause),
            ))
            .push(control(
                "media-skip-forward-symbolic",
                media.can_go_next.then_some(media::Command::Next),
            ))
            .push(widget::Space::new().width(Length::Fill))
            .push(lyrics)
            .spacing(spacing.space_m)
            .align_y(Alignment::Center)
            .into()
    }

    /// The line being sung and the next one, under the title; empty without synced lyrics.
    fn lyrics_snippet(&self, media: &MediaState) -> Element<'_, Message> {
        use crate::modules::lyrics::{self, Lyrics};
        let spacing = cosmic::theme::spacing();
        let Some(Lyrics::Synced(lines)) = self.lyrics.as_ref().and_then(|(_, l)| l.as_ref())
        else {
            return widget::Space::new().into();
        };
        let position = media.position_at(Instant::now());
        let Some(current) = lyrics::current_line(lines, position) else {
            return widget::Space::new().into();
        };
        let line = |index: usize| {
            lines
                .get(index)
                .map(|(_, t)| if t.is_empty() { "♪".to_owned() } else { t.clone() })
        };
        let mut column = widget::Column::new().spacing(spacing.space_xxxs);
        if let Some(now) = line(current) {
            column = column.push(
                widget::text::heading(now)
                    .class(cosmic::theme::Text::Custom(accent_text))
                    .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1))),
            );
        }
        if let Some(next) = line(current + 1) {
            column = column.push(
                widget::text::caption(next)
                    .class(cosmic::theme::Text::Custom(secondary_text))
                    .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1))),
            );
        }
        column.into()
    }

    /// Lyrics of the playing track; synced ones scroll with the song.
    fn lyrics_panel(&self, media: &MediaState) -> Element<'_, Message> {
        use crate::modules::lyrics::Lyrics;
        let note = |text: String| -> Element<'_, Message> {
            container(widget::text::body(text).class(cosmic::theme::Text::Custom(secondary_text)))
                .width(Length::Fill)
                .height(Length::Fixed(LYRICS_HEIGHT))
                .align_y(Alignment::Center)
                .into()
        };
        match self.lyrics.as_ref().and_then(|(_, l)| l.as_ref()) {
            None => note(fl!("lyrics-loading")),
            Some(Lyrics::NotFound) => note(fl!("lyrics-not-found")),
            Some(Lyrics::Instrumental) => note(fl!("lyrics-instrumental")),
            Some(Lyrics::Plain(lines)) => widget::Column::new()
                .push(
                    widget::scrollable(widget::text::body(lines.join("\n")).width(Length::Fill))
                        .height(Length::Fixed(LYRICS_HEIGHT)),
                )
                .push(self.lyrics_source())
                .width(Length::Fill)
                .into(),
            Some(Lyrics::Synced(lines)) => widget::Column::new()
                .push(self.synced_lyrics(lines, media))
                .push(self.lyrics_source())
                .width(Length::Fill)
                .into(),
        }
    }

    fn lyrics_source(&self) -> Element<'_, Message> {
        widget::text::caption(fl!("lyrics-source"))
            .class(cosmic::theme::Text::Custom(secondary_text))
            .into()
    }

    /// The song's lines in a column that keeps the line being sung in the
    /// middle: it grows and brightens as it arrives, the others shrink and
    /// fade with their distance, and the column slides up when the line changes.
    fn synced_lyrics(&self, lines: &[(Duration, String)], media: &MediaState) -> Element<'_, Message> {
        let position = media.position_at(Instant::now());
        let (current, progress) = lyrics::scroll_progress(lines, position);
        // Where the middle of the view is, in lines: it moves from the previous line to the current one.
        let middle = match current {
            Some(k) => k as f32 - 1.0 + progress,
            None => -1.0,
        }
        .max(-1.0);
        let base = middle.floor();
        let first = base as i64 - LYRICS_ROWS as i64 / 2;
        let theme = cosmic::theme::active();
        let on: Color = theme.cosmic().background(theme.transparent).on.into();
        let mut column = widget::Column::new();
        for row in 0..=LYRICS_ROWS as i64 {
            let index = first + row;
            let distance = (index as f32 - middle).abs();
            let line = usize::try_from(index).ok().and_then(|i| lines.get(i).map(|l| (i, l)));
            let element: Element<'_, Message> = match line {
                Some((_, (start, text))) => {
                    let text = if text.is_empty() { "♪" } else { text.as_str() };
                    let emphasis = (1.0 - distance).clamp(0.0, 1.0);
                    let size = LYRICS_SMALL + (LYRICS_LARGE - LYRICS_SMALL) * emphasis;
                    // Long lines get a smaller size rather than a second line.
                    let size = size * (28.0 / text.chars().count().max(1) as f32).clamp(0.72, 1.0);
                    let alpha = 0.4 + 0.6 * emphasis;
                    let label = widget::text(text.to_owned())
                        .size(size)
                        .font(if emphasis > 0.5 {
                            cosmic::font::semibold()
                        } else {
                            cosmic::font::default()
                        })
                        .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1)))
                        .class(cosmic::theme::Text::Color(faded(Color {
                            a: alpha.clamp(0.12, 1.0),
                            ..on
                        })));
                    // A click on a line plays from there.
                    widget::mouse_area(
                        container(label)
                            .width(Length::Fill)
                            .height(Length::Fixed(LYRICS_LINE))
                            .align_y(Alignment::Center),
                    )
                    .on_press(Message::SeekTo(*start))
                    .into()
                }
                None => widget::Space::new()
                    .width(Length::Fill)
                    .height(Length::Fixed(LYRICS_LINE))
                    .into(),
            };
            column = column.push(element);
        }
        container(fade(column, 1.0, 1.0).shifted((middle - base) * LYRICS_LINE))
            .width(Length::Fill)
            .height(Length::Fixed(LYRICS_HEIGHT))
            .clip(true)
            .into()
    }

    fn progress(&self, media: &MediaState) -> Element<'_, Message> {
        let spacing = cosmic::theme::spacing();
        let Some(length) = media.length else {
            // Length unknown (a stream, or a video still loading): elapsed time on an empty track.
            let track = container(widget::Space::new())
                .width(Length::Fill)
                .height(Length::Fixed(4.0))
                .class(cosmic::theme::Container::custom(|theme| {
                    plate_style(theme, theme.cosmic().radius_xs())
                }));
            return widget::Row::new()
                .push(widget::text::caption(media::format_duration(
                    media.position_at(Instant::now()),
                )))
                .push(track)
                .push(widget::text::caption("--:--"))
                .spacing(spacing.space_xs)
                .align_y(Alignment::Center)
                .into();
        };
        let length_s = length.as_secs_f32();
        let position_s = self
            .seeking
            .unwrap_or_else(|| media.position_at(Instant::now()).as_secs_f32())
            .min(length_s);
        let position = std::time::Duration::from_secs_f32(position_s);

        let bar: Element<'_, Message> = if media.can_seek && media.track_id.is_some() {
            widget::slider(0.0..=length_s, position_s, Message::Seek)
                .on_release(Message::SeekReleased)
                .step(0.5_f32)
                .class(slider_class())
                .into()
        } else {
            widget::determinate_linear(position_s / length_s.max(1.0)).into()
        };

        widget::Row::new()
            .push(widget::text::caption(media::format_duration(position)))
            .push(container(bar).width(Length::Fill))
            .push(widget::text::caption(format!(
                "-{}",
                media::format_duration(length.saturating_sub(position))
            )))
            .spacing(spacing.space_xs)
            .align_y(Alignment::Center)
            .into()
    }

    fn expanded_view(&self, id: window::Id, opacity: f32) -> Element<'_, Message> {
        let spacing = cosmic::theme::spacing();

        // Icon-only tabs say what they are on hover, which also names them for screen readers.
        let tab = |handle, tab: Tab, name: String| {
            widget::tooltip(
                island_button(handle, self.tab == tab, Message::SelectTab(tab)),
                widget::text::caption(name),
                widget::tooltip::Position::Bottom,
            )
        };

        let mut header = widget::Row::new();
        for section in Tab::SECTIONS.into_iter().filter(|t| self.shows(*t)) {
            let (icon, name) = match section {
                Tab::Home => (icons::house(), fl!("tab-home")),
                Tab::Media => (icons::named("audio-volume-high-symbolic"), fl!("section-media")),
                Tab::Shelf => (icons::inbox(), fl!("section-shelf")),
                Tab::Calendar => (icons::calendar(), fl!("section-calendar")),
                _ => (icons::tasks(), fl!("section-tasks")),
            };
            header = header.push(tab(icon, section, name));
        }
        let header = header
            .push(widget::Space::new().width(Length::Fill))
            .push(self.shelf_actions())
            .push(widget::text::body(self.clock_text()).font(cosmic::font::semibold()))
            .push(tab(icons::named("emblem-system-symbolic"), Tab::Settings, fl!("settings")))
            .spacing(spacing.space_xxxs)
            .align_y(Alignment::Center);

        let body: Element<'_, Message> = match self.tab {
            Tab::Media => self.media_tab(opacity),
            Tab::Shelf => self.shelf_tab(id, opacity),
            Tab::Home => self.home_tab(opacity),
            Tab::Calendar => self.calendar_tab(),
            Tab::Tasks => self.tasks_tab(),
            Tab::Settings => self.settings_tab().map(Message::Settings),
        };

        // A new tab slides in from its side and fades in on the way.
        let body: Element<'_, Message> = match self.tab_changed {
            Some((at, side)) => {
                let left = geometry::tab_slide(at, std::time::Instant::now());
                fade(body, 1.0 - left, 1.0).slid(side * left * TAB_SLIDE).into()
            }
            None => body,
        };

        widget::Column::new()
            .push(header)
            .push(body)
            .spacing(spacing.space_xs)
            .padding(spacing.space_xxs)
            .width(Length::Fixed(EXPANDED_WIDTH))
            .into()
    }

    /// Share and clear buttons for the shown shelf, in the header while it has files.
    fn shelf_actions(&self) -> Element<'_, Message> {
        let spacing = cosmic::theme::spacing();
        let mut row = widget::Row::new()
            .spacing(spacing.space_xxxs)
            .align_y(Alignment::Center);
        if self.tab == Tab::Shelf && !self.share_page {
            row = row.push(widget::tooltip(
                island_button(icons::named("edit-paste-symbolic"), false, Message::Paste),
                widget::text::caption(fl!("paste")),
                widget::tooltip::Position::Bottom,
            ));
        }
        if self.tab == Tab::Shelf && !self.shelves.current().items.is_empty() && !self.share_page {
            row = row.push(widget::tooltip(
                island_button(icons::send(), false, Message::OpenShare),
                widget::text::caption(fl!("send")),
                widget::tooltip::Position::Bottom,
            ));
            row = row.push(widget::tooltip(
                island_button(icons::trash(), false, Message::ShelfClear),
                widget::text::caption(fl!("shelf-clear")),
                widget::tooltip::Position::Bottom,
            ));
            row = row.push(widget::Space::new().width(Length::Fixed(f32::from(spacing.space_xs))));
        }
        row.into()
    }

    fn calendar_tab(&self) -> Element<'_, Message> {
        use crate::modules::calendar;
        use chrono::Datelike;

        let spacing = cosmic::theme::spacing();
        let today = self.now.date_naive();
        let first_day = calendar::weekday_from_config(self.time_config.first_day_of_week);
        let month = self.calendar_month;
        let google = self.config.google_calendar;
        const CELL: f32 = 44.0;
        const DAY: f32 = 28.0;

        let header = widget::Row::new()
            .push(
                widget::text::heading(calendar::month_title(month, self.language))
                    .class(cosmic::theme::Text::Custom(accent_text))
                    .width(Length::Fill),
            )
            .push(island_button(
                icons::named("go-previous-symbolic"),
                false,
                Message::CalendarShift(-1),
            ))
            .push(island_button(
                icons::named("go-next-symbolic"),
                false,
                Message::CalendarShift(1),
            ))
            .align_y(Alignment::Center);

        let cell = |content: Element<'static, Message>, height: f32| {
            container(content)
                // Days share the width left by the side panel.
                .width(Length::Fill)
                .height(Length::Fixed(height))
                .align_x(Alignment::Center)
                .align_y(Alignment::Center)
        };

        let mut grid = widget::Column::new().push(header).spacing(2);
        let mut names = widget::Row::new().width(Length::Fill);
        for name in calendar::weekday_names(first_day, self.language) {
            names = names.push(cell(
                widget::text::caption(name)
                    .class(cosmic::theme::Text::Custom(secondary_text))
                    .into(),
                CELL * 0.6,
            ));
        }
        grid = grid.push(names);
        let row_height = if google { CELL * 0.92 } else { CELL * 0.8 };
        for week in calendar::month_grid(month.year(), month.month(), first_day) {
            let mut row = widget::Row::new().width(Length::Fill);
            for date in week {
                let Some(date) = date else {
                    row = row.push(cell(widget::Space::new().into(), row_height));
                    continue;
                };
                let label = widget::text::body(date.day().to_string()).font(cosmic::font::semibold());
                let weekend =
                    matches!(date.weekday(), chrono::Weekday::Sat | chrono::Weekday::Sun);
                let selected = google && date == self.calendar.selected;
                let size = if google { DAY } else { CELL * 0.75 };
                let label: Element<'static, Message> = if date == today {
                    container(label.class(cosmic::theme::Text::Custom(|theme| {
                        cosmic::iced::widget::text::Style {
                            color: Some(faded(theme.cosmic().on_accent_color())),
                            ..Default::default()
                        }
                    })))
                    .width(Length::Fixed(size))
                    .height(Length::Fixed(size))
                    .align_x(Alignment::Center)
                    .align_y(Alignment::Center)
                    .class(cosmic::theme::Container::custom(|theme| container::Style {
                        background: Some(faded(theme.cosmic().accent_color()).into()),
                        border: cosmic::iced::Border {
                            radius: CELL.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }))
                    .into()
                } else if selected {
                    container(label)
                        .width(Length::Fixed(size))
                        .height(Length::Fixed(size))
                        .align_x(Alignment::Center)
                        .align_y(Alignment::Center)
                        .class(cosmic::theme::Container::custom(|theme| {
                            plate_style(theme, [CELL; 4])
                        }))
                        .into()
                } else {
                    let label = if weekend {
                        label.class(cosmic::theme::Text::Custom(secondary_text))
                    } else {
                        label
                    };
                    // As big as the circle of today, so the hover is that circle too.
                    container(label)
                        .width(Length::Fixed(size))
                        .height(Length::Fixed(size))
                        .align_x(Alignment::Center)
                        .align_y(Alignment::Center)
                        .into()
                };
                if !google {
                    row = row.push(cell(label, row_height));
                    continue;
                }
                // A dot under days that have events; days are buttons that pick the day.
                let dot: Element<'static, Message> = if self.calendar.on(date).is_empty() {
                    widget::Space::new().height(Length::Fixed(4.0)).into()
                } else {
                    container(widget::Space::new())
                        .width(Length::Fixed(4.0))
                        .height(Length::Fixed(4.0))
                        .class(cosmic::theme::Container::custom(|theme| container::Style {
                            background: Some(faded(theme.cosmic().accent_color()).into()),
                            border: cosmic::iced::Border {
                                radius: 2.0.into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        }))
                        .into()
                };
                // The button is the day's circle, so its hover is round like the selection.
                let day = widget::Column::new()
                    .push(
                        button::custom(label)
                            .padding(0)
                            .class(round_button_class())
                            .on_press(Message::CalendarSelect(date)),
                    )
                    .push(dot)
                    .spacing(2)
                    .align_x(Alignment::Center);
                row = row.push(cell(day.into(), row_height));
            }
            grid = grid.push(row);
        }

        let side: Element<'_, Message> = if google {
            self.events_panel()
        } else {
            widget::Column::new()
                .push(
                    widget::text::heading(clock::format(&self.now, "%A", self.language))
                        .class(cosmic::theme::Text::Custom(accent_text)),
                )
                .push(widget::text::title1(today.day().to_string()))
                .push(widget::text::body(calendar::month_title(today, self.language)))
                .push(
                    widget::text::caption(fl!("week", number = today.iso_week().week().to_string()))
                        .class(cosmic::theme::Text::Custom(secondary_text)),
                )
                .spacing(spacing.space_xxxs)
                .width(Length::Fixed(180.0))
                .into()
        };

        widget::Row::new()
            .push(side)
            .push(grid.width(Length::Fill))
            .spacing(spacing.space_s)
            .into()
    }

    /// The side of the calendar with Google Calendar on: the events of the picked day, or the form for a new one.
    fn events_panel(&self) -> Element<'_, Message> {
        use crate::modules::calendar;
        use crate::modules::google_calendar::When;
        use chrono::Datelike;

        let spacing = cosmic::theme::spacing();
        let day = self.calendar.selected;
        const WIDTH: f32 = 230.0;
        const LIST_HEIGHT: f32 = 150.0;

        let heading = widget::Row::new()
            .push(
                widget::Column::new()
                    .push(
                        widget::text::heading(clock::format(
                            &day.and_hms_opt(12, 0, 0).unwrap_or_default().and_utc(),
                            "%A",
                            self.language,
                        ))
                        .class(cosmic::theme::Text::Custom(accent_text)),
                    )
                    .push(
                        widget::Row::new()
                            .push(widget::text::title1(day.day().to_string()))
                            .push(
                                widget::text::caption(calendar::month_title(day, self.language))
                                    .class(cosmic::theme::Text::Custom(secondary_text)),
                            )
                            .spacing(spacing.space_xxs)
                            .align_y(Alignment::End),
                    )
                    .width(Length::Fill),
            )
            .push(island_button(
                icons::named("list-add-symbolic"),
                self.calendar.form.is_some(),
                Message::EventNew,
            ))
            .align_y(Alignment::Start);

        let body: Element<'_, Message> = if let Some(form) = &self.calendar.form {
            let time_input = |value: &str, on_input: fn(String) -> Message| {
                widget::text_input("12:00", value.to_owned())
                    .on_input(on_input)
                    .on_submit(|_| Message::EventSubmit)
                    .width(Length::Fixed(64.0))
            };
            let mut column = widget::Column::new()
                .push(
                    widget::text_input(fl!("event-title"), form.title.clone())
                        .id(super::events::event_title_input_id())
                        .on_input(Message::EventTitle)
                        .on_submit(|_| Message::EventSubmit)
                        .width(Length::Fill),
                )
                .push(
                    widget::Row::new()
                        .push(widget::text::body(fl!("event-all-day")).width(Length::Fill))
                        .push(widget::toggler(form.all_day).on_toggle(Message::EventAllDay))
                        .align_y(Alignment::Center),
                )
                .spacing(spacing.space_xxs);
            if !form.all_day {
                column = column.push(
                    widget::Row::new()
                        .push(widget::text::caption(fl!("event-from")))
                        .push(time_input(&form.start, Message::EventStart))
                        .push(widget::text::caption(fl!("event-to")))
                        .push(time_input(&form.end, Message::EventEnd))
                        .spacing(spacing.space_xxs)
                        .align_y(Alignment::Center),
                );
            }
            if let Some(error) = &form.error {
                column = column.push(
                    widget::text::caption(error.clone())
                        .class(cosmic::theme::Text::Custom(error_text)),
                );
            }
            column
                .push(
                    widget::Row::new()
                        .push(widget::Space::new().width(Length::Fill))
                        .push(widget::button::text(fl!("event-cancel")).on_press(Message::EventCancel))
                        .push(
                            widget::button::suggested(fl!("event-add"))
                                .on_press_maybe((!form.creating).then_some(Message::EventSubmit)),
                        )
                        .spacing(spacing.space_xxs),
                )
                // As tall as it needs, so the buttons are always there.
                .into()
        } else if let Some(note) = self.calendar_note() {
            container(widget::text::caption(note).class(cosmic::theme::Text::Custom(secondary_text)))
                .height(Length::Fixed(LIST_HEIGHT))
                .into()
        } else {
            let events = self.calendar.on(day);
            if events.is_empty() {
                container(
                    widget::text::body(fl!("events-none"))
                        .class(cosmic::theme::Text::Custom(secondary_text)),
                )
                .height(Length::Fixed(LIST_HEIGHT))
                .into()
            } else {
                let mut list = widget::Column::new().spacing(spacing.space_xxs);
                for event in events {
                    let when = match (event.start, event.end) {
                        (When::At(start), When::At(end)) => {
                            format!("{} – {}", start.format("%H:%M"), end.format("%H:%M"))
                        }
                        _ => fl!("event-all-day"),
                    };
                    let bar = container(widget::Space::new())
                        .width(Length::Fixed(3.0))
                        .height(Length::Fill)
                        .class(cosmic::theme::Container::custom(|theme| container::Style {
                            background: Some(faded(theme.cosmic().accent_color()).into()),
                            border: cosmic::iced::Border {
                                radius: 2.0.into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        }));
                    list = list.push(
                        widget::Row::new()
                            .push(bar)
                            .push(
                                widget::Column::new()
                                    .push(
                                        widget::text::body(if event.title.is_empty() {
                                            "—".to_owned()
                                        } else {
                                            event.title.clone()
                                        })
                                        .font(cosmic::font::semibold())
                                        .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1))),
                                    )
                                    .push(
                                        widget::text::caption(when)
                                            .class(cosmic::theme::Text::Custom(secondary_text)),
                                    )
                                    .width(Length::Fill),
                            )
                            .spacing(spacing.space_xs)
                            .height(Length::Fixed(40.0)),
                    );
                }
                widget::scrollable(list.padding([0, spacing.space_xs, 0, 0]))
                    .height(Length::Fixed(LIST_HEIGHT))
                    .into()
            }
        };

        widget::Column::new()
            .push(heading)
            .push(body)
            .spacing(spacing.space_xs)
            .width(Length::Fixed(WIDTH))
            .into()
    }

    /// Why no events show, when Google Calendar can't be reached.
    fn calendar_note(&self) -> Option<String> {
        use crate::modules::google_calendar::Error;
        match self.calendar.error.as_ref()? {
            Error::NoAccount => Some(fl!("events-sign-in")),
            Error::NoPermission => Some(fl!("google-no-permission")),
            Error::Other(error) => Some(fl!("google-error", error = error.clone())),
        }
    }

    fn shelf_tab(&self, id: window::Id, opacity: f32) -> Element<'_, Message> {
        if self.share_page {
            return self.share_page_view();
        }
        let spacing = cosmic::theme::spacing();
        let receiving = self.surfaces.get(&id).is_some_and(|s| s.drag_over);
        let items = &self.shelves.current().items;

        let files: Element<'_, Message> = if items.is_empty() {
            container(
                widget::Column::new()
                    .push(widget::icon(icons::inbox()).size(32))
                    .push(
                        widget::text::body(if receiving {
                            fl!("shelf-drop-here")
                        } else {
                            fl!("shelf-empty")
                        })
                            .class(cosmic::theme::Text::Custom(secondary_text)),
                    )
                    .spacing(spacing.space_xxs)
                    .align_x(Alignment::Center),
            )
            .width(Length::Fill)
            .height(Length::Fixed(PREVIEW + 76.0))
            .align_x(Alignment::Center)
            .align_y(Alignment::Center)
            .into()
        } else {
            let mut tiles = widget::Row::new()
                .spacing(spacing.space_s)
                .align_y(Alignment::Start);
            for item in items {
                let tile = self.shelf_tile(id, item, opacity);
                // Files that just landed grow into place.
                let tile = match self.landed.get(&item.uri) {
                    Some(at) => {
                        let (fade_in, scale) = geometry::content_transition(*at, Instant::now());
                        fade(tile, fade_in, 0.6 + 0.4 * (scale - 0.8) / 0.2).into()
                    }
                    None => tile,
                };
                tiles = tiles.push(tile);
            }
            widget::scrollable(
                container(tiles).padding([spacing.space_xxs, spacing.space_xxs, spacing.space_xs, spacing.space_xxs]),
            )
            .direction(cosmic::iced::widget::scrollable::Direction::Horizontal(
                cosmic::iced::widget::scrollable::Scrollbar::new()
                    .width(4)
                    .scroller_width(4),
            ))
            .width(Length::Fill)
            .height(Length::Fixed(PREVIEW + 76.0))
            .into()
        };

        widget::Column::new()
            .push(self.shelf_switcher(id))
            .push(files)
            .spacing(spacing.space_xs)
            .into()
    }

    /// Devices to send the shown shelf to: LocalSend devices nearby and paired
    /// KDE Connect devices, each with how sending to it goes.
    fn share_page_view(&self) -> Element<'_, Message> {
        use crate::modules::localsend::SendEvent;
        let spacing = cosmic::theme::spacing();
        let shelf = self.shelves.current();

        let header = widget::Row::new()
            .push(island_button(
                icons::named("go-previous-symbolic"),
                false,
                Message::CloseShare,
            ))
            .push(widget::text::heading(fl!(
                "send-shelf",
                count = shelf.items.len(),
                shelf = shelf.name.clone()
            )))
            .spacing(spacing.space_xxs)
            .align_y(Alignment::Center);

        let status = |key: &str| -> Option<String> {
            Some(match self.sends.get(key)? {
                SendEvent::Waiting => fl!("send-waiting"),
                SendEvent::Progress { done, total } => {
                    fl!("send-progress", done = done.to_string(), total = total.to_string())
                }
                SendEvent::Done => fl!("send-done"),
                SendEvent::Rejected => fl!("send-rejected"),
                SendEvent::Failed(error) => fl!("send-failed", error = error.clone()),
            })
        };
        let busy = |key: &str| {
            matches!(
                self.sends.get(key),
                Some(SendEvent::Waiting | SendEvent::Progress { .. })
            )
        };

        let device_row = |icon: &'static str,
                          protocol: cosmic::widget::icon::Handle,
                          name: String,
                          detail: String,
                          key: &str,
                          on_press: Message| {
            let mut row = widget::Row::new()
                .push(widget::icon(icons::named(icon)).size(28))
                .push(
                    widget::Column::new()
                        .push(widget::text::body(name))
                        .push(
                            widget::text::caption(detail)
                                .class(cosmic::theme::Text::Custom(secondary_text)),
                        )
                        .width(Length::Fill),
                )
                .spacing(spacing.space_xs)
                .align_y(Alignment::Center);
            // Which protocol this device is reached over.
            row = row.push(widget::icon(protocol).size(16));
            if busy(key) {
                row = row.push(widget::indeterminate_circular().size(18.0));
            }
            if let Some(status) = status(key) {
                row = row.push(widget::text::caption(status));
            }
            button::custom(row)
                .padding([spacing.space_xxs, spacing.space_xs])
                .width(Length::Fill)
                .class(button_class(false))
                .on_press_maybe((!busy(key)).then_some(on_press))
        };

        // One list of both kinds, by name; each row says which protocol it uses.
        let mut entries: Vec<(String, Element<'_, Message>)> = Vec::new();
        for device in &self.devices {
            let detail = match &device.info.device_model {
                Some(model) => format!("{model} · LocalSend"),
                None => "LocalSend".to_owned(),
            };
            entries.push((
                device.info.alias.to_lowercase(),
                device_row(
                    device.icon(),
                    icons::localsend(),
                    device.info.alias.clone(),
                    detail,
                    &device.info.fingerprint,
                    Message::SendTo(device.info.fingerprint.clone()),
                )
                .into(),
            ));
        }
        for (index, target) in self.share_targets.iter().enumerate() {
            let crate::modules::shelf::ShareTarget::KdeConnect { id, name, .. } = target;
            entries.push((
                name.to_lowercase(),
                device_row(
                    target.icon(),
                    icons::kdeconnect(),
                    name.clone(),
                    "KDE Connect".to_owned(),
                    id,
                    Message::Share(index),
                )
                .into(),
            ));
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        let mut list = widget::Column::new().spacing(spacing.space_xxxs);
        for (_, row) in entries {
            list = list.push(row);
        }

        // Keeps looking while the page is open; says so below the devices.
        let searching = widget::Row::new()
            .push(widget::indeterminate_circular().size(16.0))
            .push(
                widget::text::caption(fl!("send-searching"))
                    .class(cosmic::theme::Text::Custom(secondary_text)),
            )
            .spacing(spacing.space_xxs)
            .align_y(Alignment::Center);

        widget::Column::new()
            .push(header)
            .push(
                widget::scrollable(list.push(container(searching).padding([
                    spacing.space_xxs,
                    spacing.space_xs,
                ])))
                .height(Length::Fixed(PREVIEW + 110.0)),
            )
            .spacing(spacing.space_xs)
            .into()
    }

    /// Names of the shelves to switch between, the one being renamed as a text field.
    fn shelf_switcher(&self, id: window::Id) -> Element<'_, Message> {
        let spacing = cosmic::theme::spacing();
        let mut row = widget::Row::new()
            .spacing(spacing.space_xxxs)
            .align_y(Alignment::Center);
        for (index, shelf) in self.shelves.list.iter().enumerate() {
            if let Some((editing, name)) = &self.renaming
                && *editing == index
                && !self.shelves.is_history(index)
            {
                row = row.push(
                    widget::text_input("", name.clone())
                        .id(super::shelf_name_input_id())
                        .on_input(Message::ShelfRenameInput)
                        .on_submit(|_| Message::ShelfRenameCommit)
                        .width(Length::Fixed(140.0)),
                );
                row = row.push(island_button(
                    icons::named("object-select-symbolic"),
                    false,
                    Message::ShelfRenameCommit,
                ));
                if self.shelves.list.len() > 1 && !self.shelves.is_history(index) {
                    row = row.push(island_button(
                        icons::trash(),
                        false,
                        Message::ShelfDelete(index),
                    ));
                }
                continue;
            }
            let selected = index == self.shelves.current;
            let count = shelf.items.len();
            // The history is named in the language of the moment, not of when it was made.
            let name = if self.shelves.is_history(index) {
                fl!("clipboard")
            } else {
                shelf.name.clone()
            };
            let label = if count > 0 {
                format!("{name} · {count}")
            } else {
                name
            };
            let chip = widget::mouse_area(
                button::custom(widget::text::body(label))
                    .padding([spacing.space_xxxs, spacing.space_xs])
                    .class(chip_class(selected))
                    .on_press(Message::ShelfSelect(index)),
            )
            .on_double_click(if self.shelves.is_history(index) {
                Message::ShelfSelect(index)
            } else {
                Message::ShelfRenameStart(index)
            });
            // Dragging a file over a shelf's name switches to it, so it lands there.
            let chip = widget::dnd_destination::dnd_destination_for_data::<Clip, _>(
                chip,
                move |clip, _| Message::Dropped(id, clip),
            )
            .on_enter(move |_, _, _| Message::DragOverShelf(id, index));
            row = row.push(chip);
            if selected && self.shelves.list.len() > 1 && !self.shelves.is_history(index) {
                if self.confirm_delete == Some(index) {
                    row = row.push(
                        button::custom(widget::text::body(fl!("delete-shelf-confirm")))
                            .padding([spacing.space_xxxs, spacing.space_xs])
                            .class(cosmic::theme::Button::Destructive)
                            .on_press(Message::ShelfDelete(index)),
                    );
                    row = row.push(island_button(
                        icons::named("window-close-symbolic"),
                        false,
                        Message::ShelfDeleteCancel,
                    ));
                } else {
                    row = row.push(widget::tooltip(
                        button::custom(widget::icon(icons::trash()).size(14))
                            .padding(spacing.space_xxxs)
                            .class(button_class(false))
                            .on_press(Message::ShelfDelete(index)),
                        widget::text::caption(fl!("delete-shelf")),
                        widget::tooltip::Position::Bottom,
                    ));
                }
            }
        }
        row = row.push(island_button(
            icons::named("list-add-symbolic"),
            false,
            Message::ShelfCreate,
        ));
        widget::scrollable(row)
            .direction(cosmic::iced::widget::scrollable::Direction::Horizontal(
                cosmic::iced::widget::scrollable::Scrollbar::new()
                    .width(0)
                    .scroller_width(0),
            ))
            .width(Length::Fill)
            .into()
    }

    fn preview_source(&self, item: &Item) -> Preview {
        if let Some(text) = &item.text {
            return Preview::Text(text.chars().take(400).collect());
        }
        match self.thumbnails.get(&item.uri).and_then(Option::as_ref) {
            Some(thumb) => Preview::Thumbnail(thumb.clone()),
            None => Preview::Icon(file_icon(item)),
        }
    }

    fn shelf_tile(&self, id: window::Id, item: &Item, opacity: f32) -> Element<'_, Message> {
        let uri = item.uri.clone();
        let preview = self.preview_source(item);
        let name = item.name.clone();
        // Snippets show what they say instead of their file name.
        let name = match (&item.text, &self.copied) {
            (Some(_), Some((copied, at)))
                if *copied == item.uri && at.elapsed() < std::time::Duration::from_millis(1500) =>
            {
                fl!("copied")
            }
            (Some(_), _) => fl!("text-snippet"),
            (None, _) => name,
        };
        let (drag_preview, drag_name) = (preview.clone(), name.clone());
        let text = item.text.clone();
        let (shelf, from_uri) = (self.shelves.current, uri.clone());
        let tile = widget::dnd_source::<Message, DragOut>(
            tile(preview, name, opacity).map(|()| unreachable!("tiles send no messages")),
        )
        // Files go out as files; snippets also as text, for editors.
        .drag_content(move || DragOut {
            uris: crate::modules::shelf::UriList(vec![uri.clone()]),
            text: text.clone(),
        })
        // What follows the pointer while dragging: the same preview and name,
        // with the grabbed point of the file kept under the pointer.
        .drag_icon(move |offset| {
            (
                tile(drag_preview.clone(), drag_name.clone(), 1.0),
                cosmic::iced::advanced::widget::tree::State::None,
                -offset,
            )
        })
        // Dragging a file out leaves it on the shelf: other programs get a copy.
        // Dropped on another shelf of the island, it moves there.
        .action(DndAction::Copy)
        .on_start(Some(Message::InternalDragStart(shelf, from_uri)))
        .on_finish(Some(Message::InternalDragEnd))
        .on_cancel(Some(Message::InternalDragEnd))
        .window(id);

        // A click copies a snippet back to the clipboard, or opens a file.
        let tile = widget::mouse_area(tile).on_release(Message::OpenItem(item.uri.clone()));

        // A small remove button on the preview's top-right corner.
        let remove = button::custom(widget::icon(icons::named("window-close-symbolic")).size(12))
            .padding(3)
            .class(remove_button_class())
            .on_press(Message::ShelfRemove(item.uri.clone()));

        cosmic::iced::widget::Stack::new()
            .push(container(tile).padding([6, 6, 0, 0]))
            .push(
                container(remove)
                    .width(Length::Fixed(PREVIEW + 12.0))
                    .align_x(Alignment::End),
            )
            .into()
    }

    pub(super) fn placeholder(&self, text: String) -> Element<'_, Message> {
        container(widget::text::body(text).class(cosmic::theme::Text::Custom(secondary_text)))
            .width(Length::Fill)
            .height(Length::Fixed(220.0))
            .align_x(Alignment::Center)
            .align_y(Alignment::Center)
            .into()
    }
}

/// What a shelf file shows: its thumbnail, or the icon of its type.
#[derive(Clone)]
enum Preview {
    Thumbnail(widget::image::Handle),
    Icon(icon::Handle),
    Text(String),
}

/// A file on the shelf: preview with its name below, no wider than the preview.
fn tile(preview: Preview, name: String, opacity: f32) -> Element<'static, ()> {
    let spacing = cosmic::theme::spacing();
    let preview: Element<'static, ()> = match preview {
        Preview::Thumbnail(handle) => widget::image(handle)
            .width(Length::Fixed(PREVIEW))
            .height(Length::Fixed(PREVIEW))
            .border_radius(cosmic::theme::active().cosmic().radius_s())
            .opacity(opacity)
            .into(),
        Preview::Icon(handle) => widget::icon(handle).size(PREVIEW as u16).into(),
        Preview::Text(text) => container(
            widget::text::caption(text)
                .wrapping(Wrapping::WordOrGlyph)
                .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(5))),
        )
        .width(Length::Fixed(PREVIEW))
        .height(Length::Fixed(PREVIEW))
        .padding(6)
        .clip(true)
        .class(cosmic::theme::Container::custom(|theme| {
            let cosmic = theme.cosmic();
            let on = cosmic.background(theme.transparent).on;
            container::Style {
                background: Some(faded(Color { a: 0.1, ..on.into() }).into()),
                border: cosmic::iced::Border {
                    radius: cosmic.radius_s().into(),
                    width: 1.0,
                    color: faded(Color { a: 0.15, ..on.into() }),
                },
                ..Default::default()
            }
        }))
        .into(),
    };
    widget::Column::new()
        .push(preview)
        .push(
            widget::text::caption(name)
                // Break long names inside words too, so they never outgrow the preview.
                .wrapping(Wrapping::WordOrGlyph)
                .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(2)))
                .align_x(Alignment::Center)
                .width(Length::Fixed(PREVIEW)),
        )
        .spacing(spacing.space_xxxs)
        .align_x(Alignment::Center)
        .width(Length::Fixed(PREVIEW))
        .into()
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    Color {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
}

/// Full-color icon for a shelf file: a folder, or the icon of its MIME type.
fn file_icon(item: &Item) -> icon::Handle {
    if item.is_dir() {
        return icon::from_name("folder").handle();
    }
    let mime = item
        .path
        .as_deref()
        .and_then(|p| mime_guess::from_path(p).first());
    let names: Vec<String> = mime
        .map(|m| {
            vec![
                m.essence_str().replace('/', "-"),
                format!("{}-x-generic", m.type_()),
            ]
        })
        .unwrap_or_default();
    let mut named = icon::from_name("text-x-generic");
    if let Some(first) = names.first() {
        named = icon::from_name(first.clone()).fallback(Some(icon::IconFallback::Names(
            names
                .iter()
                .skip(1)
                .cloned()
                .chain(["text-x-generic".to_owned()])
                .map(Into::into)
                .collect(),
        )));
    }
    named.handle()
}

fn control(icon: &'static str, command: Option<media::Command>) -> Element<'static, Message> {
    island_button_sized(icons::named(icon), false, command.map(Message::MediaCommand), 40)
}

/// An icon button whose colors fade with the island's content.
pub(super) fn island_button(
    handle: icon::Handle,
    selected: bool,
    on_press: Message,
) -> Element<'static, Message> {
    island_button_sized(handle, selected, Some(on_press), 20)
}

pub(super) fn island_button_sized(
    handle: icon::Handle,
    selected: bool,
    on_press: Option<Message>,
    icon_size: u16,
) -> Element<'static, Message> {
    let padding = cosmic::theme::spacing().space_xxs;
    button::custom(widget::icon(handle).size(icon_size))
        .padding(padding)
        .class(button_class(selected))
        .on_press_maybe(on_press)
        .into()
}

/// Flat buttons: a faint plate on hover and when selected, like the design's tabs.
pub(super) fn button_class(selected: bool) -> cosmic::theme::Button {
    fn style(theme: &cosmic::Theme, plate: f32) -> button::Style {
        let cosmic = theme.cosmic();
        let on = cosmic.background(theme.transparent).on;
        button::Style {
            background: (plate > 0.0)
                .then(|| faded(Color { a: plate, ..on.into() }).into()),
            border_radius: cosmic.radius_s().into(),
            icon_color: Some(faded(on)),
            text_color: Some(faded(on)),
            ..button::Style::new()
        }
    }
    let base = if selected { 0.15 } else { 0.0 };
    cosmic::theme::Button::Custom {
        active: Box::new(move |_, theme| style(theme, base)),
        disabled: Box::new(move |theme| {
            let mut s = style(theme, 0.0);
            s.icon_color = s.icon_color.map(|c| Color { a: c.a * 0.4, ..c });
            s
        }),
        hovered: Box::new(move |_, theme| style(theme, base.max(0.1))),
        pressed: Box::new(move |_, theme| style(theme, 0.2)),
    }
}

/// A flat round button: a faint circle on hover, as round as the selection in the calendar.
fn round_button_class() -> cosmic::theme::Button {
    fn style(theme: &cosmic::Theme, plate: f32) -> button::Style {
        let cosmic = theme.cosmic();
        let on = cosmic.background(theme.transparent).on;
        button::Style {
            background: (plate > 0.0).then(|| faded(Color { a: plate, ..on.into() }).into()),
            border_radius: 999.0.into(),
            icon_color: Some(faded(on)),
            text_color: Some(faded(on)),
            ..button::Style::new()
        }
    }
    cosmic::theme::Button::Custom {
        active: Box::new(|_, theme| style(theme, 0.0)),
        disabled: Box::new(|theme| style(theme, 0.0)),
        hovered: Box::new(|_, theme| style(theme, 0.1)),
        pressed: Box::new(|_, theme| style(theme, 0.2)),
    }
}

/// COSMIC's standard slider, with colors that fade with the island's content.
fn slider_class() -> cosmic::theme::iced::Slider {
    use cosmic::iced::widget::slider;
    fn style(theme: &cosmic::Theme, handle: u16) -> slider::Style {
        let cosmic = theme.cosmic();
        slider::Style {
            rail: slider::Rail {
                backgrounds: (
                    faded(cosmic.accent.base).into(),
                    faded(cosmic.palette.neutral_6).into(),
                ),
                width: 4.0,
                border: cosmic::iced::Border {
                    radius: cosmic.corner_radii.radius_xs.into(),
                    ..Default::default()
                },
            },
            handle: slider::Handle {
                shape: slider::HandleShape::Rectangle {
                    height: handle,
                    width: handle,
                    border_radius: cosmic.corner_radii.radius_m.into(),
                },
                background: faded(cosmic.accent.base).into(),
                border_color: Color::TRANSPARENT,
                border_width: 0.0,
            },
            breakpoint: slider::Breakpoint {
                color: faded(cosmic.on_bg_color()),
            },
        }
    }
    cosmic::theme::iced::Slider::Custom {
        active: std::rc::Rc::new(|theme| style(theme, 20)),
        hovered: std::rc::Rc::new(|theme| style(theme, 24)),
        dragging: std::rc::Rc::new(|theme| style(theme, 24)),
    }
}

/// Shelf names: a plate in the text color when selected or hovered.
fn chip_class(selected: bool) -> cosmic::theme::Button {
    fn style(theme: &cosmic::Theme, plate: f32, selected: bool) -> button::Style {
        let cosmic = theme.cosmic();
        let on = cosmic.background(theme.transparent).on;
        let text = if selected {
            Color::from(on)
        } else {
            Color { a: 0.7, ..on.into() }
        };
        button::Style {
            background: (plate > 0.0).then(|| faded(Color { a: plate, ..on.into() }).into()),
            border_radius: cosmic.radius_xl().into(),
            text_color: Some(faded(text)),
            icon_color: Some(faded(text)),
            ..button::Style::new()
        }
    }
    let base = if selected { 0.15 } else { 0.0 };
    cosmic::theme::Button::Custom {
        active: Box::new(move |_, theme| style(theme, base, selected)),
        disabled: Box::new(move |theme| style(theme, 0.0, selected)),
        hovered: Box::new(move |_, theme| style(theme, base.max(0.1), selected)),
        pressed: Box::new(move |_, theme| style(theme, 0.2, selected)),
    }
}

/// Round remove button on a plate, readable over any thumbnail.
pub(super) fn remove_button_class() -> cosmic::theme::Button {
    fn style(theme: &cosmic::Theme, plate: f32) -> button::Style {
        let cosmic = theme.cosmic();
        let bg = cosmic.background(theme.transparent);
        button::Style {
            background: Some(faded(Color { a: plate, ..bg.base.into() }).into()),
            border_radius: 999.0.into(),
            border_width: 1.0,
            border_color: faded(bg.divider),
            icon_color: Some(faded(bg.on)),
            ..button::Style::new()
        }
    }
    cosmic::theme::Button::Custom {
        active: Box::new(|_, theme| style(theme, 0.85)),
        disabled: Box::new(|theme| style(theme, 0.5)),
        hovered: Box::new(|_, theme| style(theme, 1.0)),
        pressed: Box::new(|_, theme| style(theme, 1.0)),
    }
}

/// Secondary text: the theme's text color at reduced opacity.
pub fn secondary_text(theme: &cosmic::Theme) -> cosmic::iced::widget::text::Style {
    let on = theme.cosmic().background(theme.transparent).on;
    cosmic::iced::widget::text::Style {
        color: Some(faded(Color { a: 0.7, ..on.into() })),
        ..Default::default()
    }
}

pub(super) fn accent_text(theme: &cosmic::Theme) -> cosmic::iced::widget::text::Style {
    cosmic::iced::widget::text::Style {
        color: Some(faded(theme.cosmic().accent_text_color())),
        ..Default::default()
    }
}

pub(super) fn error_text(theme: &cosmic::Theme) -> cosmic::iced::widget::text::Style {
    cosmic::iced::widget::text::Style {
        color: Some(faded(theme.cosmic().destructive_text_color())),
        ..Default::default()
    }
}

/// A faint plate in the text color, e.g. behind a missing album cover.
pub(super) fn plate_style(theme: &cosmic::Theme, radius: [f32; 4]) -> container::Style {
    let on = theme.cosmic().background(theme.transparent).on;
    container::Style {
        background: Some(faded(Color { a: 0.12, ..on.into() }).into()),
        border: cosmic::iced::Border {
            radius: radius.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

/// The island itself; `glow` (0–1) tints its border and background with the
/// accent color, while a file is held over it and right after a drop.
fn pill_style(
    theme: &cosmic::Theme,
    size: Size,
    expanded: bool,
    glow: f32,
    solid: bool,
) -> container::Style {
    let cosmic = theme.cosmic();
    let bg = cosmic.background(theme.transparent && !solid);
    let radius = geometry::pill_radius(Radii::from_theme(cosmic).pill(expanded), size);
    let accent = Color::from(cosmic.accent_color());
    let border_width = 1.0 + glow;
    let border_color = mix(bg.divider.into(), accent, glow);
    let base = Color::from(bg.base);
    container::Style {
        background: Some(mix(base, Color { a: base.a, ..accent }, 0.12 * glow).into()),
        text_color: Some(bg.on.into()),
        icon_color: Some(bg.on.into()),
        border: cosmic::iced::Border {
            radius: radius.into(),
            width: border_width,
            color: border_color,
        },
        shadow: Default::default(),
        snap: true,
    }
}
