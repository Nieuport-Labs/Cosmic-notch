// SPDX-License-Identifier: GPL-3.0-only

//! Which outputs a fullscreen window covers, so the island can step aside
//! for videos and games like the panel does.
//!
//! Uses its own Wayland connection: toplevel info is not part of iced's.

use std::collections::BTreeSet;

use cosmic::cctk::cosmic_protocols::toplevel_info::v1::client::zcosmic_toplevel_handle_v1::State as ToplevelState;
use cosmic::cctk::sctk::output::{OutputHandler, OutputState};
use cosmic::cctk::sctk::registry::{ProvidesRegistryState, RegistryState};
use cosmic::cctk::toplevel_info::{ToplevelInfoHandler, ToplevelInfoState};
use cosmic::cctk::wayland_client::globals::registry_queue_init;
use cosmic::cctk::wayland_client::protocol::wl_output;
use cosmic::cctk::wayland_client::{Connection, QueueHandle};
use cosmic::cctk::wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1;
use cosmic::cctk::wayland_protocols::ext::workspace::v1::client::ext_workspace_handle_v1;
use cosmic::cctk::workspace::{WorkspaceHandler, WorkspaceState};
use cosmic::cctk::sctk;
use cosmic::iced::futures::{SinkExt, Stream, channel::mpsc};

/// Outputs with a fullscreen window in front, by name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Covered {
    pub outputs: BTreeSet<String>,
    /// The focused window is fullscreen, for an island that follows the active output.
    pub focused: bool,
}

/// Yields what is covered whenever it changes.
pub fn watch() -> impl Stream<Item = Covered> {
    cosmic::iced::stream::channel(4, |mut output: mpsc::Sender<Covered>| async move {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        std::thread::Builder::new()
            .name("fullscreen-watch".into())
            .spawn(move || {
                if let Err(err) = run(tx) {
                    tracing::warn!("cannot follow fullscreen windows: {err}");
                }
            })
            .ok();
        while let Some(covered) = rx.recv().await {
            if output.send(covered).await.is_err() {
                break;
            }
        }
        // Without the watcher nothing is covered; stay quiet.
        std::future::pending::<()>().await;
    })
}

fn run(tx: tokio::sync::mpsc::UnboundedSender<Covered>) -> Result<(), Box<dyn std::error::Error>> {
    let conn = Connection::connect_to_env()?;
    let (globals, mut queue) = registry_queue_init(&conn)?;
    let qh = queue.handle();
    let registry_state = RegistryState::new(&globals);
    let toplevels =
        ToplevelInfoState::try_new(&registry_state, &qh).ok_or("no toplevel info protocol")?;
    let mut state = State {
        outputs: OutputState::new(&globals, &qh),
        workspaces: WorkspaceState::new(&registry_state, &qh),
        toplevels,
        registry_state,
        sent: None,
        tx,
    };
    loop {
        queue.blocking_dispatch(&mut state)?;
        if state.tx.is_closed() {
            return Ok(());
        }
    }
}

struct State {
    registry_state: RegistryState,
    outputs: OutputState,
    workspaces: WorkspaceState,
    toplevels: ToplevelInfoState,
    sent: Option<Covered>,
    tx: tokio::sync::mpsc::UnboundedSender<Covered>,
}

impl State {
    fn covered(&self) -> Covered {
        let active = |workspace: &ext_workspace_handle_v1::ExtWorkspaceHandleV1| {
            self.workspaces
                .workspace_info(workspace)
                .is_some_and(|w| w.state.contains(ext_workspace_handle_v1::State::Active))
        };
        let mut covered = Covered::default();
        for toplevel in self.toplevels.toplevels() {
            let shown = toplevel.state.contains(&ToplevelState::Fullscreen)
                && !toplevel.state.contains(&ToplevelState::Minimized)
                // Without workspace info, assume it is on the shown workspace.
                && (toplevel.workspace.is_empty() || toplevel.workspace.iter().any(active));
            if !shown {
                continue;
            }
            covered.focused |= toplevel.state.contains(&ToplevelState::Activated);
            covered.outputs.extend(
                toplevel
                    .output
                    .iter()
                    .filter_map(|o| self.outputs.info(o)?.name),
            );
        }
        covered
    }

    fn publish(&mut self) {
        let covered = self.covered();
        if self.sent.as_ref() != Some(&covered) {
            self.sent = Some(covered.clone());
            let _ = self.tx.send(covered);
        }
    }
}

impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    sctk::registry_handlers!(OutputState);
}

impl OutputHandler for State {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        self.publish();
    }

    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        self.publish();
    }
}

impl ToplevelInfoHandler for State {
    fn toplevel_info_state(&mut self) -> &mut ToplevelInfoState {
        &mut self.toplevels
    }

    fn new_toplevel(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &ExtForeignToplevelHandleV1) {}

    fn update_toplevel(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &ExtForeignToplevelHandleV1) {}

    fn toplevel_closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &ExtForeignToplevelHandleV1) {
        self.publish();
    }

    fn info_done(&mut self, _: &Connection, _: &QueueHandle<Self>) {
        self.publish();
    }
}

impl WorkspaceHandler for State {
    fn workspace_state(&mut self) -> &mut WorkspaceState {
        &mut self.workspaces
    }

    fn done(&mut self) {
        self.publish();
    }
}

sctk::delegate_output!(State);
sctk::delegate_registry!(State);
cosmic::cctk::delegate_toplevel_info!(State);
cosmic::cctk::delegate_workspace!(State);
