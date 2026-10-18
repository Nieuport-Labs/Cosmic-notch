// SPDX-License-Identifier: GPL-3.0-only

//! The panel applet: keeps room in the panel where the island floats, and
//! starts the island daemon through D-Bus activation when it is not running.

use cosmic::app::{Core, Task};
use cosmic::iced::{Length, Subscription};
use cosmic::{Element, widget};

pub const APPLET_ID: &str = "io.github.jiricepelka.CosmicExtIsland.Applet";

pub fn run() -> cosmic::iced::Result {
    cosmic::applet::run::<Applet>(())
}

pub struct Applet {
    core: Core,
    /// Width of the compact island, 0 while the daemon is unavailable.
    island_width: u32,
}

#[derive(Debug, Clone)]
pub enum Message {
    IslandWidth(u32),
}

impl cosmic::Application for Applet {
    type Executor = cosmic::executor::Default;
    type Flags = ();
    type Message = Message;
    const APP_ID: &'static str = APPLET_ID;

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, _flags: ()) -> (Self, Task<Message>) {
        (
            Self {
                core,
                island_width: 0,
            },
            Task::none(),
        )
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::IslandWidth(width) => self.island_width = width,
        }
        Task::none()
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::run(crate::ipc::watch_compact_width).map(Message::IslandWidth)
    }

    fn view(&self) -> Element<'_, Message> {
        let (_, height) = self.core.applet.suggested_size(true);
        let (_, padding) = self.core.applet.suggested_padding(true);
        // Transparent room as wide as the island; the island itself is drawn
        // over the panel by the daemon.
        widget::Space::new()
            .width(Length::Fixed(self.island_width.max(1) as f32))
            .height(Length::Fixed(f32::from(height + 2 * padding)))
            .into()
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}
