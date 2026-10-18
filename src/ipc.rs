// SPDX-License-Identifier: GPL-3.0-only

//! D-Bus interface between the island daemon and its panel applet.
//!
//! The daemon owns [`BUS_NAME`] (which also keeps a second daemon from
//! starting) and publishes the width of the compact island, so the applet can
//! keep that much room free in the panel. `Toggle` opens or closes the island,
//! e.g. from a keyboard shortcut:
//!
//! ```sh
//! busctl --user call io.github.jiricepelka.CosmicExtIsland \
//!     /io/github/jiricepelka/CosmicExtIsland io.github.jiricepelka.CosmicExtIsland Toggle
//! ```

use std::time::Duration;

use cosmic::iced::futures::{SinkExt, Stream, StreamExt, channel::mpsc};

pub const BUS_NAME: &str = "io.github.jiricepelka.CosmicExtIsland";
pub const OBJECT_PATH: &str = "/io/github/jiricepelka/CosmicExtIsland";

struct Interface {
    compact_width: u32,
    requests: tokio::sync::mpsc::UnboundedSender<Event>,
}

#[zbus::interface(name = "io.github.jiricepelka.CosmicExtIsland")]
impl Interface {
    /// Expands the island, or collapses it when it is expanded.
    fn toggle(&self) {
        let _ = self.requests.send(Event::Toggle);
    }

    /// Width of the compact island in logical pixels.
    #[zbus(property)]
    fn compact_width(&self) -> u32 {
        self.compact_width
    }
}

#[derive(Debug, Clone)]
pub enum Event {
    /// The daemon owns its name; keep the connection to publish updates.
    Ready(zbus::Connection),
    /// Another daemon already runs.
    AlreadyRunning,
    Toggle,
}

/// Serves the daemon's interface and yields requests made through it.
pub fn serve() -> impl Stream<Item = Event> {
    cosmic::iced::stream::channel(8, |mut output: mpsc::Sender<Event>| async move {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let interface = Interface {
            compact_width: 0,
            requests: tx,
        };
        let connection = async {
            zbus::connection::Builder::session()?
                .serve_at(OBJECT_PATH, interface)?
                .build()
                .await
        }
        .await;
        let connection = match connection {
            Ok(c) => c,
            Err(err) => {
                tracing::error!("D-Bus unavailable: {err}");
                return std::future::pending().await;
            }
        };
        let flags = zbus::fdo::RequestNameFlags::DoNotQueue.into();
        match connection.request_name_with_flags(BUS_NAME, flags).await {
            Ok(zbus::fdo::RequestNameReply::PrimaryOwner) => {
                let _ = output.send(Event::Ready(connection.clone())).await;
            }
            Ok(_) | Err(zbus::Error::NameTaken) => {
                let _ = output.send(Event::AlreadyRunning).await;
                return std::future::pending().await;
            }
            Err(err) => tracing::error!("cannot own {BUS_NAME}: {err}"),
        }
        while let Some(event) = rx.recv().await {
            if output.send(event).await.is_err() {
                break;
            }
        }
        std::future::pending().await
    })
}

/// Publishes a new compact width.
pub async fn set_compact_width(connection: zbus::Connection, width: u32) {
    let result = async {
        let iface = connection
            .object_server()
            .interface::<_, Interface>(OBJECT_PATH)
            .await?;
        let mut guard = iface.get_mut().await;
        if guard.compact_width != width {
            guard.compact_width = width;
            guard
                .compact_width_changed(iface.signal_emitter())
                .await?;
        }
        zbus::Result::Ok(())
    }
    .await;
    if let Err(err) = result {
        tracing::warn!("failed to publish compact width: {err}");
    }
}

#[zbus::proxy(
    interface = "io.github.jiricepelka.CosmicExtIsland",
    default_service = "io.github.jiricepelka.CosmicExtIsland",
    default_path = "/io/github/jiricepelka/CosmicExtIsland"
)]
pub trait Island {
    fn toggle(&self) -> zbus::Result<()>;

    #[zbus(property)]
    fn compact_width(&self) -> zbus::Result<u32>;
}

/// Connection to the compositor that the panel hands its applets (with
/// `X-HostWaylandDisplay`), for the daemon: unlike a sandboxed app, it may
/// draw over the panel. One connection serves one client, so it goes to the
/// first daemon this applet starts.
static PRIVILEGED_SOCKET: std::sync::Mutex<Option<std::os::fd::OwnedFd>> =
    std::sync::Mutex::new(None);

/// Takes the privileged socket from the environment; call once at startup.
pub fn take_privileged_socket() {
    use std::os::fd::FromRawFd;
    let Some(fd) = std::env::var("X_PRIVILEGED_WAYLAND_SOCKET")
        .ok()
        .and_then(|fd| fd.parse::<std::os::fd::RawFd>().ok())
    else {
        return;
    };
    // SAFETY: the panel passes this descriptor to the applet alone, and
    // nothing else in the applet takes it.
    let fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) };
    *PRIVILEGED_SOCKET.lock().unwrap_or_else(|e| e.into_inner()) = Some(fd);
}

/// Starts the daemon on the privileged socket, if this applet still has it.
fn start_daemon() -> bool {
    use std::os::fd::AsRawFd;
    use std::os::unix::process::CommandExt;
    let Some(socket) = PRIVILEGED_SOCKET
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
    else {
        return false;
    };
    let Some(daemon) = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join("cosmic-ext-island")))
    else {
        return false;
    };
    let fd = socket.as_raw_fd();
    let mut command = std::process::Command::new(daemon);
    command
        .env("WAYLAND_SOCKET", fd.to_string())
        .env_remove("X_PRIVILEGED_WAYLAND_SOCKET");
    // SAFETY: only async-signal-safe calls between fork and exec.
    unsafe {
        command.pre_exec(move || {
            // Hand the socket over, and end with the applet that started it.
            let flags = libc::fcntl(fd, libc::F_GETFD);
            libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC);
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            Ok(())
        });
    }
    match command.spawn() {
        Ok(_) => true,
        Err(err) => {
            tracing::error!("cannot start the island: {err}");
            false
        }
    }
}

/// Follows the daemon's compact width. When no daemon runs, starts one: on the
/// panel's privileged socket if the applet has it, otherwise through D-Bus
/// activation. Yields 0 while it is unavailable.
pub fn watch_compact_width() -> impl Stream<Item = u32> {
    cosmic::iced::stream::channel(8, |mut output: mpsc::Sender<u32>| async move {
        loop {
            let result: zbus::Result<()> = async {
                let connection = zbus::Connection::session().await?;
                let dbus = zbus::fdo::DBusProxy::new(&connection).await?;
                let name = zbus::names::BusName::try_from(BUS_NAME)?;
                if !dbus.name_has_owner(name).await? && start_daemon() {
                    // Give it a moment to take its name.
                    for _ in 0..50 {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        let name = zbus::names::BusName::try_from(BUS_NAME)?;
                        if dbus.name_has_owner(name).await? {
                            break;
                        }
                    }
                }
                let proxy = IslandProxy::new(&connection).await?;
                let mut changes = proxy.receive_compact_width_changed().await;
                // Calling the daemon activates it when it does not run yet.
                let _ = output.send(proxy.compact_width().await?).await;
                while let Some(change) = changes.next().await {
                    if let Ok(width) = change.get().await {
                        let _ = output.send(width).await;
                    }
                }
                Ok(())
            }
            .await;
            if let Err(err) = result {
                tracing::debug!("island daemon unavailable: {err}");
            }
            let _ = output.send(0).await;
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    })
}
