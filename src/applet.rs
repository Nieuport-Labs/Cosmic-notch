// SPDX-License-Identifier: GPL-3.0-only

//! The panel applet is the island: see [`crate::app`].

pub const APPLET_ID: &str = "io.github.jiricepelka.CosmicExtIsland.Applet";

pub fn run() -> cosmic::iced::Result {
    let mut flags = crate::app::Flags::default();
    // For development and screenshots: start open on a tab.
    if let Ok(tab) = std::env::var("COSMIC_EXT_ISLAND_EXPAND") {
        use crate::app::Tab;
        flags.expand = match tab.as_str() {
            "media" => Some(Tab::Media),
            "shelf" => Some(Tab::Shelf),
            "calendar" => Some(Tab::Calendar),
            "home" => Some(Tab::Home),
            "tasks" => Some(Tab::Tasks),
            "settings" => Some(Tab::Settings),
            _ => None,
        };
    }
    crate::app::run(flags)
}
