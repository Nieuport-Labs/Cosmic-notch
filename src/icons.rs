// SPDX-License-Identifier: GPL-3.0-only

//! Icons: the user's icon theme first, bundled Lucide icons where COSMIC has none.

use cosmic::widget::icon::{self, Handle};

/// A symbolic icon from the system icon theme, tinted like text.
pub fn named(name: &'static str) -> Handle {
    icon::from_name(name).symbolic(true).handle()
}

fn bundled(svg: &'static [u8]) -> Handle {
    let mut handle = icon::from_svg_bytes(svg);
    handle.symbolic = true;
    handle
}

pub fn inbox() -> Handle {
    bundled(include_bytes!("../res/icons/inbox-symbolic.svg"))
}

pub fn calendar() -> Handle {
    bundled(include_bytes!("../res/icons/calendar-clock-symbolic.svg"))
}

pub fn trash() -> Handle {
    bundled(include_bytes!("../res/icons/trash-symbolic.svg"))
}

pub fn send() -> Handle {
    bundled(include_bytes!("../res/icons/send-symbolic.svg"))
}

/// Marks devices reached over LocalSend.
pub fn localsend() -> Handle {
    bundled(include_bytes!("../res/icons/localsend-symbolic.svg"))
}

/// Marks devices reached over KDE Connect.
pub fn kdeconnect() -> Handle {
    bundled(include_bytes!("../res/icons/kdeconnect-symbolic.svg"))
}

pub fn house() -> Handle {
    bundled(include_bytes!("../res/icons/house-symbolic.svg"))
}

pub fn tasks() -> Handle {
    bundled(include_bytes!("../res/icons/list-todo-symbolic.svg"))
}
