// SPDX-License-Identifier: GPL-3.0-only

pub mod applet;
pub mod config;
pub mod daemon;
pub mod i18n;
pub mod icons;
pub mod ipc;
pub mod island;
pub mod modules;
pub mod panel;
pub mod thumbnail;

/// Whether the island runs as a Flatpak, where host programs are started
/// through `flatpak-spawn --host`.
pub fn in_flatpak() -> bool {
    std::path::Path::new("/.flatpak-info").exists()
}
