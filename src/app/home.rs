// SPDX-License-Identifier: GPL-3.0-only

//! The home tab: how busy the computer is.

use std::collections::VecDeque;
use std::time::Duration;

use cosmic::iced::advanced::text::{Ellipsize, EllipsizeHeightLimit};
use cosmic::iced::{Alignment, Color, Length};
use cosmic::widget::{self, container};
use cosmic::Element;

use super::view::{accent_text, button_class, plate_style, secondary_text};
use super::{Island, Message};
use crate::fl;
use crate::island::fade::faded;
use crate::modules::system::{self, Charge, Disk};

/// Height of a card on the home tab.
const CARD_HEIGHT: f32 = 160.0;
const GRAPH_HEIGHT: f32 = 40.0;
/// Height of the mini player and the usage card below the graphs.
const SMALL_CARD_HEIGHT: f32 = 128.0;
/// Cover of the mini player.
const COVER: f32 = SMALL_CARD_HEIGHT - 32.0;

impl Island {
    pub(super) fn home_tab(&self, opacity: f32) -> Element<'_, Message> {
        let spacing = cosmic::theme::spacing();
        let monitor = system::monitor();
        let locale = self.locale;
        let bytes = |b: u64| system::format_bytes(b as f64, locale);

        // CPU: use now, its history and the package temperature.
        let mut cpu_side = widget::Row::new().spacing(spacing.space_xxs);
        if let Some(temperature) = monitor.temperature {
            cpu_side = cpu_side.push(secondary(format!("{temperature:.0} °C")));
        }
        let cpu = card(
            fl!("cpu"),
            cpu_side.into(),
            widget::Column::new()
                .push(
                    widget::Row::new()
                        .push(widget::text::title2(percent(monitor.cpu_now())).font(cosmic::font::bold()))
                        .push(secondary(match monitor.frequency {
                            Some(ghz) => format!(
                                "{} · {}",
                                fl!("cores", count = monitor.cores),
                                system::format_ghz(ghz, locale)
                            ),
                            None => fl!("cores", count = monitor.cores),
                        }))
                        .spacing(spacing.space_xs)
                        .align_y(Alignment::Center),
                )
                .push(graph(&monitor.cpu, 1.0, GRAPH_HEIGHT, false))
                .spacing(spacing.space_xxs)
                .into(),
        );

        // Network: download above the middle line, upload below it.
        let rx = monitor.rx.back().copied().unwrap_or(0.0);
        let tx = monitor.tx.back().copied().unwrap_or(0.0);
        let peak = monitor
            .rx
            .iter()
            .chain(&monitor.tx)
            .copied()
            .fold(10_000.0, f64::max);
        let rx_history: VecDeque<f32> = monitor.rx.iter().map(|v| (v / peak) as f32).collect();
        let tx_history: VecDeque<f32> = monitor.tx.iter().map(|v| (v / peak) as f32).collect();
        let rate = |icon: &'static str, value: f64| {
            widget::Row::new()
                .push(widget::icon(crate::icons::named(icon)).size(16))
                .push(widget::text::body(format!(
                    "{}/s",
                    system::format_bytes(value, locale)
                )))
                .spacing(spacing.space_xxxs)
                .align_y(Alignment::Center)
        };
        let network = card(
            fl!("network"),
            widget::Space::new().into(),
            widget::Column::new()
                .push(
                    widget::Row::new()
                        .push(rate("go-down-symbolic", rx))
                        .push(rate("go-up-symbolic", tx))
                        .spacing(spacing.space_s),
                )
                .push(
                    widget::Column::new()
                        .push(graph(&rx_history, 1.0, GRAPH_HEIGHT / 2.0, false))
                        .push(graph(&tx_history, 1.0, GRAPH_HEIGHT / 2.0, true))
                        .spacing(1),
                )
                .spacing(spacing.space_xxs)
                .into(),
        );

        // Memory and disks, one line and bar each.
        let memory = monitor.memory;
        let share = |used: u64, total: u64| used as f32 / total.max(1) as f32;
        let usage = |name: String, detail: String, share: f32| {
            widget::Column::new()
                .push(
                    widget::Row::new()
                        .push(widget::text::body(name).width(Length::Fill))
                        .push(secondary(detail))
                        .align_y(Alignment::Center),
                )
                .push(meter(share))
                .spacing(2)
        };
        let memory_share = share(memory.used(), memory.total);
        let mut usage_body = widget::Column::new()
            .push(usage(
                fl!("memory"),
                format!(
                    "{} · {}",
                    fl!("used-of", used = bytes(memory.used()), total = bytes(memory.total)),
                    percent(memory_share)
                ),
                memory_share,
            ))
            .spacing(spacing.space_xs);
        for disk in monitor.disks.iter().take(2) {
            usage_body = usage_body.push(usage(
                disk_name(disk),
                fl!("free", free = bytes(disk.available)),
                share(disk.used(), disk.total),
            ));
        }
        let usage = small_card(usage_body.into());

        let mut second_row = widget::Row::new().spacing(spacing.space_xs);
        if let Some(player) = self.mini_player(opacity) {
            second_row = second_row.push(player);
        }
        second_row = second_row.push(usage);

        // Battery and uptime below the cards.
        let mut footer = widget::Row::new()
            .spacing(spacing.space_s)
            .align_y(Alignment::Center);
        if let Some(battery) = monitor.battery {
            let state = match battery.charge {
                Charge::Charging => fl!("battery-charging"),
                Charge::Discharging => fl!("battery-discharging"),
                Charge::Full => fl!("battery-full"),
            };
            footer = footer.push(
                widget::Row::new()
                    .push(widget::icon(widget::icon::from_name(battery_icon(battery)).symbolic(true).handle()).size(16))
                    .push(widget::text::caption(format!("{} % · {state}", battery.percent)))
                    .spacing(spacing.space_xxxs)
                    .align_y(Alignment::Center),
            );
        }
        footer = footer
            .push(widget::Space::new().width(Length::Fill))
            .push(secondary(fl!("uptime", time = format_uptime(monitor.uptime))));

        widget::Column::new()
            .push(
                widget::Row::new()
                    .push(cpu)
                    .push(network)
                    .spacing(spacing.space_xs),
            )
            .push(second_row)
            .push(container(footer).padding([0, spacing.space_xxs]))
            .spacing(spacing.space_xs)
            .into()
    }
}

impl Island {
    /// What plays, with its controls; a click on it opens the media tab.
    fn mini_player(&self, opacity: f32) -> Option<Element<'_, Message>> {
        use crate::modules::media::{Command, Status};
        let media = self.media.as_ref()?;
        let spacing = cosmic::theme::spacing();
        let mut title = widget::Row::new()
            .push(
                widget::text::heading(media.title.clone())
                    .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1)))
                    .width(Length::Fill),
            )
            .spacing(spacing.space_xxs)
            .align_y(Alignment::Center);
        if media.status == Status::Playing && self.bars_enabled() {
            title = title.push(self.bars(14.0));
        }
        let controls = widget::Row::new()
            .push(control(
                "media-skip-backward-symbolic",
                media.can_go_previous.then_some(Command::Previous),
                20,
            ))
            .push(control(
                if media.status == Status::Playing {
                    "media-playback-pause-symbolic"
                } else {
                    "media-playback-start-symbolic"
                },
                Some(Command::PlayPause),
                20,
            ))
            .push(control(
                "media-skip-forward-symbolic",
                media.can_go_next.then_some(Command::Next),
                20,
            ))
            .spacing(spacing.space_xxs)
            .align_y(Alignment::Center);
        let text = widget::mouse_area(
            widget::Column::new()
                .push(title)
                .push(
                    secondary(media.artists.clone())
                        .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1))),
                )
                .spacing(2)
                .width(Length::Fill),
        )
        .on_press(Message::SelectTab(super::Tab::Media));
        let played = media.length.map_or(0.0, |length| {
            media.position_at(std::time::Instant::now()).as_secs_f32() / length.as_secs_f32().max(1.0)
        });
        Some(small_card_filled(
            played,
            widget::Row::new()
                .push(
                    widget::mouse_area(self.cover(COVER, false, opacity))
                        .on_press(Message::SelectTab(super::Tab::Media)),
                )
                .push(
                    widget::Column::new()
                        .push(text)
                        .push(widget::Space::new().height(Length::Fill))
                        .push(controls)
                        .height(Length::Fill),
                )
                .spacing(spacing.space_s)
                .into(),
        ))
    }
}

/// A compact player button; the media tab's are too wide for the card.
fn control(
    icon: &'static str,
    command: Option<crate::modules::media::Command>,
    size: u16,
) -> Element<'static, Message> {
    widget::button::custom(widget::icon(crate::icons::named(icon)).size(size))
        .padding(6)
        .class(button_class(false))
        .on_press_maybe(command.map(Message::MediaCommand))
        .into()
}

/// A card without a title, for the second row.
fn small_card(body: Element<'_, Message>) -> Element<'_, Message> {
    small_card_filled(0.0, body)
}

/// A card whose background fills from the left with `share` (0–1) in the accent color,
/// like a progress bar behind the content.
fn small_card_filled(share: f32, body: Element<'_, Message>) -> Element<'_, Message> {
    let share = share.clamp(0.0, 1.0);
    let filled = (share * 1000.0).round() as u16;
    let mut fill = widget::Row::new().height(Length::Fill);
    if filled > 0 {
        fill = fill.push(
            container(widget::Space::new())
                .width(Length::FillPortion(filled))
                .height(Length::Fill)
                .class(cosmic::theme::Container::custom(move |theme| {
                    let cosmic = theme.cosmic();
                    let r = cosmic.radius_s();
                    // Square on the right until the fill reaches the card's corner.
                    let right = if filled >= 995 { r[1] } else { 0.0 };
                    container::Style {
                        background: Some(
                            faded(Color { a: 0.22, ..cosmic.accent_color().into() }).into(),
                        ),
                        border: cosmic::iced::Border {
                            radius: [r[0], right, right, r[3]].into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }
                })),
        );
    }
    if filled < 1000 {
        fill = fill.push(widget::Space::new().width(Length::FillPortion(1000 - filled)));
    }
    container(cosmic::iced::widget::stack![
        fill,
        container(body)
            .padding(cosmic::theme::spacing().space_s)
            .width(Length::Fill)
            .height(Length::Fill),
    ])
    .width(Length::Fill)
    .height(Length::Fixed(SMALL_CARD_HEIGHT))
    .class(cosmic::theme::Container::custom(|theme| {
        plate_style(theme, theme.cosmic().radius_s())
    }))
    .into()
}

fn secondary(text: String) -> widget::Text<'static, cosmic::Theme> {
    widget::text::caption(text).font(cosmic::font::semibold()).class(cosmic::theme::Text::Custom(secondary_text))
}

fn percent(share: f32) -> String {
    format!("{:.0} %", share * 100.0)
}

/// A card with a title, something on the right of it and a body.
fn card<'a>(
    title: String,
    side: Element<'a, Message>,
    body: Element<'a, Message>,
) -> Element<'a, Message> {
    let spacing = cosmic::theme::spacing();
    container(
        widget::Column::new()
            .push(
                widget::Row::new()
                    .push(
                        widget::text::heading(title)
                            .class(cosmic::theme::Text::Custom(accent_text))
                            .width(Length::Fill),
                    )
                    .push(side)
                    .align_y(Alignment::Center),
            )
            .push(body)
            .spacing(spacing.space_xxs),
    )
    .padding(spacing.space_s)
    .width(Length::Fill)
    .height(Length::Fixed(CARD_HEIGHT))
    .class(cosmic::theme::Container::custom(|theme| {
        plate_style(theme, theme.cosmic().radius_s())
    }))
    .into()
}

/// A history as thin bars, newest on the right; `hanging` bars grow down from the top.
fn graph(history: &VecDeque<f32>, max: f32, height: f32, hanging: bool) -> Element<'static, Message> {
    let missing = system::HISTORY.saturating_sub(history.len());
    let values = std::iter::repeat_n(0.0, missing).chain(history.iter().copied());
    let mut row = widget::Row::new()
        .spacing(1)
        .height(Length::Fixed(height))
        .align_y(if hanging { Alignment::Start } else { Alignment::End });
    for value in values {
        let share = (value / max).clamp(0.0, 1.0);
        row = row.push(
            container(widget::Space::new())
                .width(Length::Fill)
                .height(Length::Fixed((share * height).max(1.0)))
                .class(cosmic::theme::Container::custom(move |theme| {
                    let accent = theme.cosmic().accent_color();
                    // Faint for an idle sample, so the baseline reads as one.
                    let alpha = if share > 0.0 { 0.35 + 0.65 * share } else { 0.25 };
                    container::Style {
                        background: Some(faded(Color { a: alpha, ..accent.into() }).into()),
                        border: cosmic::iced::Border {
                            radius: 1.0.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }
                })),
        );
    }
    row.into()
}

/// A filled bar: accent, or the destructive color when nearly full.
fn meter(share: f32) -> Element<'static, Message> {
    let share = share.clamp(0.0, 1.0);
    let filled = (share * 1000.0).round() as u16;
    let fill = container(widget::Space::new())
        .width(Length::FillPortion(filled.max(1)))
        .height(Length::Fill)
        .class(cosmic::theme::Container::custom(move |theme| {
            let cosmic = theme.cosmic();
            let color = if share >= 0.9 {
                cosmic.destructive_color()
            } else {
                cosmic.accent_color()
            };
            container::Style {
                background: Some(faded(color).into()),
                border: cosmic::iced::Border {
                    radius: 3.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            }
        }));
    let mut row = widget::Row::new().push(fill);
    if filled < 1000 {
        row = row.push(widget::Space::new().width(Length::FillPortion(1000 - filled)));
    }
    container(row.height(Length::Fill))
        .width(Length::Fill)
        .height(Length::Fixed(6.0))
        .class(cosmic::theme::Container::custom(|theme| {
            plate_style(theme, [3.0; 4])
        }))
        .into()
}

fn disk_name(disk: &Disk) -> String {
    match disk.mount.as_str() {
        "/" => fl!("disk-system"),
        "/home" => fl!("disk-home"),
        mount => mount.rsplit('/').next().unwrap_or(mount).to_string(),
    }
}

fn battery_icon(battery: system::Battery) -> String {
    let level = (u32::from(battery.percent) + 5) / 10 * 10;
    match battery.charge {
        Charge::Discharging => format!("battery-level-{level}-symbolic"),
        Charge::Charging | Charge::Full if level == 100 => "battery-level-100-charged-symbolic".into(),
        Charge::Charging | Charge::Full => format!("battery-level-{level}-charging-symbolic"),
    }
}

/// "45 min", "3 h 12 min", "2 d 4 h".
fn format_uptime(uptime: Duration) -> String {
    let minutes = uptime.as_secs() / 60;
    let (days, hours, minutes) = (minutes / 1440, minutes / 60 % 24, minutes % 60);
    if days > 0 {
        format!("{days} d {hours} h")
    } else if hours > 0 {
        format!("{hours} h {minutes} min")
    } else {
        format!("{minutes} min")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uptime_text() {
        assert_eq!(format_uptime(Duration::from_secs(45 * 60 + 30)), "45 min");
        assert_eq!(format_uptime(Duration::from_secs(3 * 3600 + 12 * 60)), "3 h 12 min");
        assert_eq!(format_uptime(Duration::from_secs(2 * 86400 + 4 * 3600)), "2 d 4 h");
    }
}
