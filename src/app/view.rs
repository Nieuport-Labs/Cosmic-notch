// SPDX-License-Identifier: GPL-3.0-only

//! What the island draws. Colors, radii, spacing and fonts all come from the
//! user's COSMIC theme; nothing here is a fixed color. Every color of the
//! island's content goes through [`faded`], so content can fade in.

use std::time::Instant;

use cosmic::iced::advanced::text::{Ellipsize, EllipsizeHeightLimit, Wrapping};
use cosmic::iced::clipboard::dnd::DndAction;
use cosmic::applet::cosmic_panel_config::PanelAnchor;
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
use crate::modules::media::{self, MediaState, Status};
use crate::modules::shelf::{Clip, DragOut, Item};

/// Width of the expanded island's content, from the design.
pub const EXPANDED_WIDTH: f32 = 640.0;
/// Gap between the panel's edges and the compact island.
const COMPACT_INSET: f32 = 4.0;
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
    /// The compact island, which is the applet in the panel.
    pub(super) fn applet_view(&self) -> Element<'_, Message> {
        let (_, thickness) = super::applet_thickness(&self.core);
        let horizontal = self.core.applet.is_horizontal();
        let pill_height = (f32::from(thickness) - 2.0 * COMPACT_INSET).max(20.0);
        let content = if horizontal {
            self.compact_view(pill_height, 1.0)
        } else {
            self.compact_column(f32::from(thickness) - 2.0 * COMPACT_INSET)
        };
        let pill = container(measure(content, Message::CompactMeasured))
            .clip(true)
            .class(cosmic::theme::Container::custom(move |theme| {
                pill_style(theme, Size::new(f32::MAX, pill_height), false, 0.0)
            }));
        let pill = widget::mouse_area(pill)
            .on_enter(Message::CompactEntered)
            .on_exit(Message::CompactLeft)
            .on_press(Message::CompactPressed)
            .on_middle_press(Message::MediaCommand(media::Command::PlayPause));
        let island = self.id;
        let pill = widget::dnd_destination::dnd_destination_for_data::<Clip, _>(
            pill,
            move |clip, _| Message::Dropped(island, clip),
        )
        .on_enter(move |_, _, _| Message::DragEntered(island))
        .on_leave(move || Message::DragLeft(island));
        // The panel gives the applet exactly the room the pill takes.
        let (width, height) = if horizontal {
            (Length::Shrink, Length::Fixed(f32::from(thickness)))
        } else {
            (Length::Fixed(f32::from(thickness)), Length::Shrink)
        };
        self.core.applet.autosize_window(
            container(pill)
                .width(width)
                .height(height)
                .align_x(Alignment::Center)
                .align_y(Alignment::Center),
        )
        .into()
    }

    /// The compact island on a panel at the side of the screen: the cover or
    /// the time, stacked.
    fn compact_column(&self, width: f32) -> Element<'_, Message> {
        let spacing = cosmic::theme::spacing();
        let mut column = widget::Column::new()
            .align_x(Alignment::Center)
            .spacing(spacing.space_xxxs)
            .padding([spacing.space_xxs, 0])
            .width(Length::Fixed(width));
        if self.media.is_some() && self.config.show_cover {
            column = column.push(self.cover((width * 0.7).round(), true, 1.0));
        }
        let now = self.now;
        column
            .push(widget::text::body(now.format("%H").to_string()))
            .push(widget::text::body(now.format("%M").to_string()))
            .into()
    }

    pub(super) fn island_view(&self, id: window::Id) -> Element<'_, Message> {
        let state = &self.state;
        let anchor = self.anchor();
        let now = Instant::now();
        let size = state.anim.at(now);
        let expanded = state.expanded;
        let (opacity, scale) = geometry::content_transition(state.content_since, now);
        let glow = state.glow(now);

        let content = if expanded {
            self.expanded_view(id, opacity)
        } else {
            self.compact_view(f32::from(super::applet_thickness(&self.core).1) - 2.0 * COMPACT_INSET, opacity)
        };
        let content = fade(content, opacity, scale);

        let pill = container(measure(content, move |s| Message::Measured(id, s)))
            .width(Length::Fixed(size.width))
            .height(Length::Fixed(size.height))
            .align_x(Alignment::Center)
            .align_y(Alignment::Start)
            .clip(true)
            .class(cosmic::theme::Container::custom(move |theme| {
                pill_style(theme, size, expanded, glow)
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

        // The pill hangs from the edge of the panel, along which it is centered.
        let (align_x, align_y) = match anchor {
            PanelAnchor::Top => (Alignment::Center, Alignment::Start),
            PanelAnchor::Bottom => (Alignment::Center, Alignment::End),
            PanelAnchor::Left => (Alignment::Start, Alignment::Center),
            PanelAnchor::Right => (Alignment::End, Alignment::Center),
        };
        container(pill)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(align_x)
            .align_y(align_y)
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
                            .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1))),
                    )
                    .max_width(self.config.title_max_width),
                );
            }
            if self.config.show_bars && media.status == Status::Playing {
                now_playing = now_playing.push(self.bars((height * 0.55).round()));
            }
            row = row.push(now_playing);
        }

        row.push(widget::text::body(self.clock_text())).into()
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

        if self.lyrics_view && self.config.lyrics {
            return widget::Column::new()
                .push(
                    widget::Row::new()
                        .push(self.cover(96.0, false, opacity))
                        .push(self.lyrics_panel(media))
                        .spacing(spacing.space_s),
                )
                .push(self.progress(media))
                .push(self.media_controls(media))
                .spacing(spacing.space_s)
                .into();
        }

        // Cover on the left; title, artist and album beside it, and the lines
        // being sung at the bottom, so the space next to the cover is used.
        const COVER: f32 = 160.0;
        let mut title_row = widget::Row::new()
            .push(
                widget::text::title3(media.title.clone())
                    .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(2)))
                    .width(Length::Fill),
            )
            .spacing(spacing.space_xs);
        if media.status == Status::Playing && self.config.show_bars {
            title_row = title_row.push(self.bars(20.0));
        }
        let mut details = widget::Column::new()
            .push(title_row)
            .push(
                widget::text::body(media.artists.clone())
                    .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1))),
            )
            .spacing(spacing.space_xxxs)
            .width(Length::Fill)
            .height(Length::Fixed(COVER));
        if let Some(album) = &media.album {
            details = details.push(
                widget::text::caption(album.clone())
                    .class(cosmic::theme::Text::Custom(secondary_text))
                    .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1))),
            );
        }
        details = details
            .push(widget::Space::new().height(Length::Fill))
            .push(self.lyrics_snippet(media));

        let info = widget::Row::new()
            .push(self.cover(COVER, false, opacity))
            .push(details)
            .spacing(spacing.space_s);

        widget::Column::new()
            .push(info)
            .push(self.progress(media))
            .push(self.media_controls(media))
            .spacing(spacing.space_s)
            .into()
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

    /// Lyrics of the playing track; synced ones follow the song line by line.
    fn lyrics_panel(&self, media: &MediaState) -> Element<'_, Message> {
        use crate::modules::lyrics::{self, Lyrics};
        let spacing = cosmic::theme::spacing();
        const HEIGHT: f32 = 150.0;

        let note = |text: String| -> Element<'_, Message> {
            container(widget::text::body(text).class(cosmic::theme::Text::Custom(secondary_text)))
                .width(Length::Fill)
                .height(Length::Fixed(HEIGHT))
                .align_y(Alignment::Center)
                .into()
        };

        let body: Element<'_, Message> = match self.lyrics.as_ref().and_then(|(_, l)| l.as_ref()) {
            None => note(fl!("lyrics-loading")),
            Some(Lyrics::NotFound) => note(fl!("lyrics-not-found")),
            Some(Lyrics::Instrumental) => note(fl!("lyrics-instrumental")),
            Some(Lyrics::Plain(lines)) => widget::scrollable(
                widget::text::body(lines.join("\n")).width(Length::Fill),
            )
            .height(Length::Fixed(HEIGHT))
            .into(),
            Some(Lyrics::Synced(lines)) => {
                let position = media.position_at(Instant::now());
                let current = lyrics::current_line(lines, position);
                let first = current.unwrap_or(0).saturating_sub(1);
                let mut column = widget::Column::new().spacing(spacing.space_xxxs);
                for (index, (_, text)) in lines.iter().enumerate().skip(first).take(5) {
                    let text = if text.is_empty() { "♪".to_owned() } else { text.clone() };
                    column = column.push(if Some(index) == current {
                        widget::text::title4(text)
                            .wrapping(Wrapping::WordOrGlyph)
                            .width(Length::Fill)
                    } else {
                        widget::text::body(text)
                            .class(cosmic::theme::Text::Custom(secondary_text))
                            .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1)))
                            .width(Length::Fill)
                    });
                }
                container(column)
                    .width(Length::Fill)
                    .height(Length::Fixed(HEIGHT))
                    .clip(true)
                    .into()
            }
        };

        widget::Column::new()
            .push(body)
            .push(
                widget::text::caption(fl!("lyrics-source"))
                    .class(cosmic::theme::Text::Custom(secondary_text)),
            )
            .width(Length::Fill)
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
            .push(widget::text::body(self.clock_text()))
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
        const CELL: f32 = 44.0;

        let today_panel = widget::Column::new()
            .push(
                widget::text::heading(clock::format(&self.now, "%A", self.locale))
                    .class(cosmic::theme::Text::Custom(accent_text)),
            )
            .push(widget::text::title1(today.day().to_string()))
            .push(widget::text::body(calendar::month_title(today, self.locale)))
            .push(
                widget::text::caption(fl!("week", number = today.iso_week().week().to_string()))
                    .class(cosmic::theme::Text::Custom(secondary_text)),
            )
            .spacing(spacing.space_xxxs)
            .width(Length::Fixed(180.0));

        let header = widget::Row::new()
            .push(
                widget::text::heading(calendar::month_title(month, self.locale))
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

        let cell = |content: Element<'static, Message>| {
            container(content)
                // Days share the width left by the today panel.
                .width(Length::Fill)
                .height(Length::Fixed(CELL * 0.8))
                .align_x(Alignment::Center)
                .align_y(Alignment::Center)
        };

        let mut grid = widget::Column::new().push(header).spacing(2);
        let mut names = widget::Row::new().width(Length::Fill);
        for name in calendar::weekday_names(first_day, self.locale) {
            names = names.push(cell(
                widget::text::caption(name)
                    .class(cosmic::theme::Text::Custom(secondary_text))
                    .into(),
            ));
        }
        grid = grid.push(names);
        for week in calendar::month_grid(month.year(), month.month(), first_day) {
            let mut row = widget::Row::new().width(Length::Fill);
            for date in week {
                let Some(date) = date else {
                    row = row.push(cell(widget::Space::new().into()));
                    continue;
                };
                let label = widget::text::body(date.day().to_string());
                let weekend =
                    matches!(date.weekday(), chrono::Weekday::Sat | chrono::Weekday::Sun);
                let label: Element<'static, Message> = if date == today {
                    container(label.class(cosmic::theme::Text::Custom(|theme| {
                        cosmic::iced::widget::text::Style {
                            color: Some(faded(theme.cosmic().on_accent_color())),
                            ..Default::default()
                        }
                    })))
                    .width(Length::Fixed(CELL * 0.75))
                    .height(Length::Fixed(CELL * 0.75))
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
                } else if weekend {
                    label.class(cosmic::theme::Text::Custom(secondary_text)).into()
                } else {
                    label.into()
                };
                row = row.push(cell(label));
            }
            grid = grid.push(row);
        }

        widget::Row::new()
            .push(today_panel)
            .push(grid.width(Length::Fill))
            .spacing(spacing.space_s)
            .into()
    }

    fn shelf_tab(&self, id: window::Id, opacity: f32) -> Element<'_, Message> {
        if self.share_page {
            return self.share_page_view();
        }
        let spacing = cosmic::theme::spacing();
        let receiving = self.state.drag_over;
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

        let mut list = widget::Column::new().spacing(spacing.space_xxxs);
        for device in &self.devices {
            let detail = match &device.info.device_model {
                Some(model) => format!("{model} · LocalSend"),
                None => "LocalSend".to_owned(),
            };
            list = list.push(device_row(
                device.icon(),
                device.info.alias.clone(),
                detail,
                &device.info.fingerprint,
                Message::SendTo(device.info.fingerprint.clone()),
            ));
        }
        for (index, target) in self.share_targets.iter().enumerate() {
            let crate::modules::shelf::ShareTarget::KdeConnect { id, name } = target;
            list = list.push(device_row(
                "phone-symbolic",
                name.clone(),
                "KDE Connect".to_owned(),
                id,
                Message::Share(index),
            ));
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
                if self.shelves.list.len() > 1 {
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
            let label = if count > 0 {
                format!("{} · {count}", shelf.name)
            } else {
                shelf.name.clone()
            };
            let chip = widget::mouse_area(
                button::custom(widget::text::body(label))
                    .padding([spacing.space_xxxs, spacing.space_xs])
                    .class(chip_class(selected))
                    .on_press(Message::ShelfSelect(index)),
            )
            .on_double_click(Message::ShelfRenameStart(index));
            // Dragging a file over a shelf's name switches to it, so it lands there.
            let chip = widget::dnd_destination::dnd_destination_for_data::<Clip, _>(
                chip,
                move |clip, _| Message::Dropped(id, clip),
            )
            .on_enter(move |_, _, _| Message::DragOverShelf(index));
            row = row.push(chip);
            if selected && self.shelves.list.len() > 1 {
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
        // Dragging a file out leaves it on the shelf: copy, never move.
        .action(DndAction::Copy)
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
        color: Some(faded(Color { a: 0.6, ..on.into() })),
        ..Default::default()
    }
}

pub(super) fn accent_text(theme: &cosmic::Theme) -> cosmic::iced::widget::text::Style {
    cosmic::iced::widget::text::Style {
        color: Some(faded(theme.cosmic().accent_text_color())),
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
fn pill_style(theme: &cosmic::Theme, size: Size, expanded: bool, glow: f32) -> container::Style {
    let cosmic = theme.cosmic();
    let bg = cosmic.background(theme.transparent);
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
