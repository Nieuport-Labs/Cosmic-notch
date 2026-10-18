// SPDX-License-Identifier: GPL-3.0-only

use cosmic_ext_island::daemon::{Flags, Tab};

fn main() -> cosmic::iced::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,cosmic_ext_island=info".into()),
        )
        .init();

    let mut flags = Flags::default();
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--expand" | "--expand=media" => flags.expand = Some(Tab::Media),
            "--expand=shelf" => flags.expand = Some(Tab::Shelf),
            "--expand=calendar" => flags.expand = Some(Tab::Calendar),
            "--expand=home" => flags.expand = Some(Tab::Home),
            "--expand=tasks" => flags.expand = Some(Tab::Tasks),
            "--expand=settings" => flags.expand = Some(Tab::Settings),
            "--help" | "-h" => {
                println!("Usage: cosmic-ext-island [--expand[=media|shelf|calendar|settings]]");
                return Ok(());
            }
            other => eprintln!("ignoring unknown argument {other}"),
        }
    }
    cosmic_ext_island::daemon::run(flags)
}
