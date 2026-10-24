// SPDX-License-Identifier: GPL-3.0-only

//! Watches what is copied, for the clipboard history, over the compositor's
//! `wlr-data-control` protocol (which a Flatpak's sandboxed connection doesn't
//! get). Only runs while the history is on.

use std::collections::HashMap;
use std::io::Read;
use std::os::fd::{AsFd, AsRawFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use cosmic::iced::futures::{SinkExt, Stream, channel::mpsc};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_registry, wl_seat::WlSeat};
use wayland_client::{Connection, Dispatch, QueueHandle, event_created_child};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1::{self, ZwlrDataControlDeviceV1},
    zwlr_data_control_manager_v1::ZwlrDataControlManagerV1,
    zwlr_data_control_offer_v1::{self, ZwlrDataControlOfferV1},
};

use super::shelf::Clip;

/// The most that is kept of one copy.
const MAX_BYTES: u64 = 20 * 1024 * 1024;
/// Marks what password managers copy; those are never kept.
const SECRET_HINT: &str = "x-kde-passwordManagerHint";

#[derive(Debug, Clone)]
pub enum Event {
    /// Something was copied.
    Copied(Clip),
    /// The watcher cannot run; the reason is for the user.
    Unavailable(String),
}

/// The connection, kept for when the watcher is started again.
static CONNECTION: Mutex<Option<Connection>> = Mutex::new(None);

fn connection() -> Result<Connection, String> {
    let mut slot = CONNECTION.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(connection) = &*slot {
        return Ok(connection.clone());
    }
    // The island's own connection belongs to its toolkit; this is a second one,
    // to the session's display. (In a Flatpak that one is sandboxed and offers no clipboard access.)
    let connection = Connection::connect_to_env().map_err(|e| e.to_string())?;
    *slot = Some(connection.clone());
    Ok(connection)
}

/// Yields everything copied for as long as it is polled.
pub fn watch() -> impl Stream<Item = Event> {
    cosmic::iced::stream::channel(16, |mut output: mpsc::Sender<Event>| async move {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let stop = Arc::new(AtomicBool::new(false));
        let guard = StopOnDrop(stop.clone());
        let thread = std::thread::Builder::new().name("clipboard-watch".into()).spawn({
            let tx = tx.clone();
            move || {
                if let Err(reason) = run(&tx, &stop) {
                    let _ = tx.send(Event::Unavailable(reason));
                }
            }
        });
        if let Err(err) = thread {
            let _ = output.send(Event::Unavailable(err.to_string())).await;
        }
        while let Some(event) = rx.recv().await {
            if output.send(event).await.is_err() {
                break;
            }
        }
        drop(guard);
        std::future::pending().await
    })
}

/// Stops the watcher thread when the subscription goes away.
struct StopOnDrop(Arc<AtomicBool>);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

struct State {
    tx: tokio::sync::mpsc::UnboundedSender<Event>,
    /// Types each offer holds, until it becomes the selection.
    offers: HashMap<wayland_client::backend::ObjectId, Vec<String>>,
    connection: Connection,
}

fn run(
    tx: &tokio::sync::mpsc::UnboundedSender<Event>,
    stop: &AtomicBool,
) -> Result<(), String> {
    let connection = connection()?;
    let (globals, mut queue) = registry_queue_init::<State>(&connection).map_err(|e| e.to_string())?;
    let qh = queue.handle();
    let seat: WlSeat = globals.bind(&qh, 1..=1, ()).map_err(|e| e.to_string())?;
    let manager: ZwlrDataControlManagerV1 = globals
        .bind(&qh, 1..=2, ())
        .map_err(|_| "the compositor does not offer clipboard access".to_owned())?;
    let _device = manager.get_data_device(&seat, &qh, ());
    let mut state = State {
        tx: tx.clone(),
        offers: HashMap::new(),
        connection: connection.clone(),
    };
    while !stop.load(Ordering::Relaxed) {
        queue.dispatch_pending(&mut state).map_err(|e| e.to_string())?;
        connection.flush().map_err(|e| e.to_string())?;
        let Some(read) = queue.prepare_read() else {
            continue;
        };
        // Wait for the compositor, but look up now and then whether to stop.
        let mut poll = libc::pollfd {
            fd: read.connection_fd().as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one valid pollfd.
        let ready = unsafe { libc::poll(&mut poll, 1, 1000) };
        if ready > 0 {
            read.read().map_err(|e| e.to_string())?;
        } else {
            drop(read);
        }
    }
    Ok(())
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlSeat, ()> for State {
    fn event(_: &mut Self, _: &WlSeat, _: wayland_client::protocol::wl_seat::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ZwlrDataControlManagerV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwlrDataControlManagerV1,
        _: <ZwlrDataControlManagerV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrDataControlDeviceV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwlrDataControlDeviceV1,
        event: zwlr_data_control_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use wayland_client::Proxy;
        match event {
            zwlr_data_control_device_v1::Event::DataOffer { id } => {
                state.offers.insert(id.id(), Vec::new());
            }
            zwlr_data_control_device_v1::Event::Selection { id } => {
                let Some(offer) = id else {
                    return;
                };
                let mimes = state.offers.remove(&offer.id()).unwrap_or_default();
                state.read(&offer, &mimes);
                // Offers that never became the selection are of no more use.
                state.offers.clear();
                offer.destroy();
            }
            _ => {}
        }
    }

    event_created_child!(State, ZwlrDataControlDeviceV1, [
        zwlr_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (ZwlrDataControlOfferV1, ()),
    ]);
}

impl Dispatch<ZwlrDataControlOfferV1, ()> for State {
    fn event(
        state: &mut Self,
        offer: &ZwlrDataControlOfferV1,
        event: zwlr_data_control_offer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use wayland_client::Proxy;
        if let zwlr_data_control_offer_v1::Event::Offer { mime_type } = event {
            state.offers.entry(offer.id()).or_default().push(mime_type);
        }
    }
}

/// The type to read, most specific first: files, an image, then text.
fn pick_mime(mimes: &[String]) -> Option<&str> {
    if mimes.iter().any(|m| m == SECRET_HINT) {
        return None;
    }
    ["text/uri-list", "image/png", "text/plain;charset=utf-8", "text/plain", "UTF8_STRING"]
        .into_iter()
        .find(|wanted| mimes.iter().any(|m| m == wanted))
}

impl State {
    /// Asks the owner of the selection for the content and sends it on when it arrives.
    fn read(&self, offer: &ZwlrDataControlOfferV1, mimes: &[String]) {
        let Some(mime) = pick_mime(mimes).map(str::to_owned) else {
            return;
        };
        let mut fds = [0; 2];
        // SAFETY: fds is two ints, as pipe2 wants.
        if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
            return;
        }
        // SAFETY: both descriptors are fresh from pipe2 and owned only here.
        let (read_end, write_end) = unsafe {
            (
                <std::os::fd::OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(fds[0]),
                <std::os::fd::OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(fds[1]),
            )
        };
        offer.receive(mime.clone(), write_end.as_fd());
        drop(write_end);
        let _ = self.connection.flush();
        let tx = self.tx.clone();
        // The owner writes when it gets to it: wait on a thread of its own.
        std::thread::spawn(move || {
            let mut data = Vec::new();
            let mut file = std::fs::File::from(read_end);
            if (&mut file).take(MAX_BYTES + 1).read_to_end(&mut data).is_err() || data.len() as u64 > MAX_BYTES {
                return;
            }
            if data.is_empty() {
                return;
            }
            if let Ok(clip) = Clip::try_from((data, mime)) {
                let _ = tx.send(Event::Copied(clip));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mimes(list: &[&str]) -> Vec<String> {
        list.iter().map(|m| (*m).to_owned()).collect()
    }

    #[test]
    fn prefers_files_then_images_then_text() {
        assert_eq!(pick_mime(&mimes(&["text/plain", "text/uri-list"])), Some("text/uri-list"));
        assert_eq!(pick_mime(&mimes(&["text/plain", "image/png"])), Some("image/png"));
        assert_eq!(pick_mime(&mimes(&["UTF8_STRING", "text/html"])), Some("UTF8_STRING"));
        assert_eq!(pick_mime(&mimes(&["image/jpeg"])), None);
    }

    #[test]
    fn never_keeps_what_password_managers_copy() {
        assert_eq!(pick_mime(&mimes(&["text/plain", SECRET_HINT])), None);
    }
}
