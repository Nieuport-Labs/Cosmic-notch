// SPDX-License-Identifier: GPL-3.0-only

//! User configuration of the island, stored with cosmic-config under [`APP_ID`].

use cosmic::cosmic_config::{self, CosmicConfigEntry, cosmic_config_derive::CosmicConfigEntry};
use serde::{Deserialize, Serialize};

pub const APP_ID: &str = "io.github.jiricepelka.CosmicExtIsland";
pub const CONFIG_VERSION: u64 = 1;

/// Parts of the expanded island that the user can hide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Section {
    Home,
    Media,
    Shelf,
    Calendar,
    Tasks,
}

/// Cards of the home page that the user can hide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HomeCard {
    Greeting,
    Cpu,
    Gpu,
    Network,
    Player,
    Storage,
}

impl HomeCard {
    pub const ALL: [HomeCard; 6] = [
        HomeCard::Greeting,
        HomeCard::Cpu,
        HomeCard::Gpu,
        HomeCard::Network,
        HomeCard::Player,
        HomeCard::Storage,
    ];
}

impl Section {
    pub const ALL: [Section; 5] = [
        Section::Home,
        Section::Media,
        Section::Shelf,
        Section::Calendar,
        Section::Tasks,
    ];
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, CosmicConfigEntry)]
#[version = 1]
pub struct IslandConfig {
    /// chrono format string for the clock; `None` follows the COSMIC time applet.
    pub clock_format: Option<String>,
    /// Delay before the island expands under the pointer, in milliseconds.
    pub hover_delay_ms: u64,
    /// Delay before the island collapses after the pointer left, in milliseconds.
    pub collapse_delay_ms: u64,
    pub show_cover: bool,
    pub show_title: bool,
    pub show_bars: bool,
    /// Maximum width of the media title in the compact island, in logical pixels.
    pub title_max_width: f32,
    /// Look lyrics up on lrclib.net (sends the artist and title there).
    pub lyrics: bool,
    /// Sections left out of the expanded island; at least one always stays.
    pub hidden_sections: Vec<Section>,
    /// Cards left out of the home page.
    pub hidden_cards: Vec<HomeCard>,
    /// Keep the tasks in step with Google Tasks, through GNOME Online Accounts.
    pub google_tasks: bool,
    /// Show and add events of Google Calendar, through GNOME Online Accounts.
    pub google_calendar: bool,
    /// No springs, fades or slides: everything jumps to where it is going.
    pub reduce_motion: bool,
}

impl IslandConfig {
    pub fn shows(&self, section: Section) -> bool {
        !self.hidden_sections.contains(&section)
    }

    pub fn shows_card(&self, card: HomeCard) -> bool {
        !self.hidden_cards.contains(&card)
    }
}

impl Default for IslandConfig {
    fn default() -> Self {
        Self {
            clock_format: None,
            hover_delay_ms: 300,
            collapse_delay_ms: 400,
            show_cover: true,
            show_title: true,
            show_bars: true,
            title_max_width: 180.0,
            // Sends what plays to a third party: only when the user asks for it.
            lyrics: false,
            hidden_sections: Vec::new(),
            hidden_cards: Vec::new(),
            google_tasks: false,
            google_calendar: false,
            reduce_motion: false,
        }
    }
}

/// Settings of the COSMIC time applet that the clock follows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, CosmicConfigEntry)]
#[version = 1]
pub struct TimeAppletConfig {
    pub military_time: bool,
    /// 0 = Monday … 6 = Sunday, as in cosmic-applet-time.
    pub first_day_of_week: u8,
    pub show_seconds: bool,
}

impl Default for TimeAppletConfig {
    fn default() -> Self {
        Self {
            military_time: true,
            first_day_of_week: 0,
            show_seconds: false,
        }
    }
}

pub const TIME_APPLET_ID: &str = "com.system76.CosmicAppletTime";

pub fn load<T: CosmicConfigEntry + Default>(id: &str, version: u64) -> T {
    match cosmic_config::Config::new(id, version) {
        Ok(config) => T::get_entry(&config).unwrap_or_else(|(errors, entry)| {
            for error in errors.iter().filter(|e| e.is_err()) {
                tracing::warn!("{id}: {error}");
            }
            entry
        }),
        Err(err) => {
            tracing::warn!("{id}: {err}");
            T::default()
        }
    }
}
