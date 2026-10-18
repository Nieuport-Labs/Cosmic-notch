// SPDX-License-Identifier: GPL-3.0-only

//! Where the top panel is, so the compact island sits inside it.

use cosmic::applet::cosmic_panel_config::{CosmicPanelOuput, PanelAnchor, PanelSize};
use cosmic::cosmic_config::{self, CosmicConfigEntry, cosmic_config_derive::CosmicConfigEntry};

pub const PANEL_CONFIG_ID: &str = "com.system76.CosmicPanel.Panel";

/// Gap between the panel edges and the compact island.
const INSET: u32 = 4;

/// The keys of the panel's config the island depends on. Read on their own
/// because cosmic-panel-config links a different cosmic-config than ours.
#[derive(Debug, Clone, PartialEq, CosmicConfigEntry)]
#[version = 1]
pub struct PanelKeys {
    pub anchor: PanelAnchor,
    pub anchor_gap: bool,
    pub margin: u16,
    pub size: PanelSize,
    pub padding: u32,
    pub output: CosmicPanelOuput,
    /// Applets in the middle of the panel, which the island covers.
    pub plugins_center: Option<Vec<String>>,
}

impl Default for PanelKeys {
    fn default() -> Self {
        Self {
            anchor: PanelAnchor::Top,
            anchor_gap: false,
            margin: 0,
            size: PanelSize::S,
            padding: 0,
            output: CosmicPanelOuput::All,
            plugins_center: None,
        }
    }
}

/// Whether the island has the middle of the panel to itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Room {
    /// The placeholder applet keeps room as wide as the island.
    Kept,
    /// Nothing is in the middle, or the panel is not at the top.
    Free,
    /// Applets in the middle are hidden under the island.
    Covered,
}

impl PanelKeys {
    pub fn load() -> Self {
        crate::config::load(PANEL_CONFIG_ID, Self::VERSION)
    }

    pub fn room(&self) -> Room {
        let center = self.plugins_center.as_deref().unwrap_or_default();
        if center.iter().any(|a| a == crate::applet::APPLET_ID) {
            Room::Kept
        } else if center.is_empty() || !matches!(self.anchor, PanelAnchor::Top) {
            Room::Free
        } else {
            Room::Covered
        }
    }

    /// Puts the placeholder applet in the middle of the panel's center applets.
    pub fn keep_room(&self) -> Result<(), cosmic_config::Error> {
        let mut center = self.plugins_center.clone().unwrap_or_default();
        if center.iter().any(|a| a == crate::applet::APPLET_ID) {
            return Ok(());
        }
        center.insert(center.len() / 2, crate::applet::APPLET_ID.to_owned());
        cosmic_config::ConfigSet::set(
            &cosmic_config::Config::new(PANEL_CONFIG_ID, Self::VERSION)?,
            "plugins_center",
            Some(center),
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PanelGeometry {
    /// Distance from the top of the output to the top of the island.
    pub top: u32,
    /// Height of the compact island.
    pub compact_height: u32,
    /// Outputs the panel is shown on.
    pub output: CosmicPanelOuput,
}

impl PanelGeometry {
    pub fn load() -> Self {
        Self::from_keys(&PanelKeys::load())
    }

    pub fn from_keys(keys: &PanelKeys) -> Self {
        if !matches!(keys.anchor, PanelAnchor::Top) {
            // No top panel: hang the island from the top edge of the output.
            return Self {
                top: INSET,
                compact_height: 32,
                output: keys.output.clone(),
            };
        }
        let gap = if keys.anchor_gap {
            u32::from(keys.margin)
        } else {
            0
        };
        let thickness = keys.size.get_applet_icon_size_with_padding(true) + 2 * keys.padding;
        let compact_height = thickness.saturating_sub(2 * INSET).max(20);
        Self {
            top: gap + thickness.saturating_sub(compact_height) / 2,
            compact_height,
            output: keys.output.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_panel_with_gap() {
        let g = PanelGeometry::from_keys(&PanelKeys {
            anchor_gap: true,
            margin: 4,
            ..PanelKeys::default()
        });
        // S panel: 20 px icon + 2 × 10 px padding = 40 px, island inset by 4 px.
        assert_eq!(g.compact_height, 32);
        assert_eq!(g.top, 8);
    }

    #[test]
    fn room_in_the_panel() {
        let keys = |center: &[&str]| PanelKeys {
            plugins_center: Some(center.iter().map(|s| s.to_string()).collect()),
            ..PanelKeys::default()
        };
        assert_eq!(keys(&[]).room(), Room::Free);
        assert_eq!(keys(&["com.system76.CosmicAppletTime"]).room(), Room::Covered);
        assert_eq!(keys(&[crate::applet::APPLET_ID]).room(), Room::Kept);
    }

    #[test]
    fn bottom_panel_hangs_from_top_edge() {
        let g = PanelGeometry::from_keys(&PanelKeys {
            anchor: PanelAnchor::Bottom,
            ..PanelKeys::default()
        });
        assert_eq!(g.top, INSET);
    }
}
