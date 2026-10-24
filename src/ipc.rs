// SPDX-License-Identifier: GPL-3.0-only

//! D-Bus interface of the island. The first island (applet instance) to start
//! owns [`BUS_NAME`]; `Toggle` opens or closes it, e.g. from a keyboard shortcut:
//!
//! ```sh
//! busctl --user call io.github.jiricepelka.CosmicExtIsland \
//!     /io/github/jiricepelka/CosmicExtIsland io.github.jiricepelka.CosmicExtIsland Toggle
//! ```

use cosmic::iced::futures::{SinkExt, Stream, channel::mpsc};

pub const BUS_NAME: &str = "io.github.jiricepelka.CosmicExtIsland";
pub const OBJECT_PATH: &str = "/io/github/jiricepelka/CosmicExtIsland";

struct Interface {
    requests: tokio::sync::mpsc::UnboundedSender<Event>,
}

#[zbus::interface(name = "io.github.jiricepelka.CosmicExtIsland")]
impl Interface {
    /// Expands the island, or collapses it when it is expanded.
    fn toggle(&self) {
        let _ = self.requests.send(Event::Toggle);
    }
}

#[derive(Debug, Clone)]
pub enum Event {
    /// This island owns the name.
    Ready,
    /// Another island (on another panel or display) owns the name.
    AlreadyRunning,
    Toggle,
}

/// Serves the daemon's interface and yields requests made through it.
pub fn serve() -> impl Stream<Item = Event> {
    cosmic::iced::stream::channel(8, |mut output: mpsc::Sender<Event>| async move {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let interface = Interface { requests: tx };
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
                let _ = output.send(Event::Ready).await;
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

/// The connection to the compositor that the panel hands its applets (with
/// `X-HostWaylandDisplay`). Unlike the applet's own, it is not sandboxed, so
/// the clipboard watcher may use protocols that need that.
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

/// Hands the privileged socket over, if the applet still has it.
pub fn privileged_socket() -> Option<std::os::fd::OwnedFd> {
    PRIVILEGED_SOCKET
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
}
