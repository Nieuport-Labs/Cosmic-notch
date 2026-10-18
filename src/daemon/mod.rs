// SPDX-License-Identifier: GPL-3.0-only

//! The island daemon: one overlay layer surface per output, drawn over the top panel.

mod home;
mod settings;
mod sync;
mod todo;
mod view;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use chrono::{DateTime, Local, Locale};
use cosmic::app::{Core, Task};
use cosmic::cctk::sctk::reexports::client::protocol::wl_output::WlOutput;
use cosmic::cctk::sctk::shell::wlr_layer::{Anchor, KeyboardInteractivity, Layer};
use cosmic::cosmic_config;
use cosmic::iced::event::{PlatformSpecific, wayland};
use cosmic::iced::keyboard::{self, key::Named};
use cosmic::iced::platform_specific::runtime::wayland::layer_surface::{
    IcedMargin, IcedOutput, SctkLayerSurfaceSettings,
};
use cosmic::iced::platform_specific::shell::commands::{
    blur::blur, layer_surface::set_keyboard_interactivity,
};
use cosmic::iced::{Color, Size, Subscription, window};
use cosmic::surface::action::{app_layer_shell, destroy_layer_shell};
use cosmic::Element;

use crate::config::{self, APP_ID, IslandConfig, Outputs, TimeAppletConfig};
use crate::island::geometry::{self, SizeAnimation};
use crate::modules::clock;
use crate::modules::google_tasks::Change;
use crate::modules::lyrics;
use crate::modules::media::{self, MediaState};
use crate::modules::shelf::{self, Clip, ShareTarget, UriList};
use crate::panel::{PANEL_CONFIG_ID, PanelGeometry, PanelKeys};

/// Options for development and screenshots.
#[derive(Debug, Clone, Default)]
pub struct Flags {
    /// Start expanded and pinned on this tab.
    pub expand: Option<Tab>,
}

pub fn run(flags: Flags) -> cosmic::iced::Result {
    crate::i18n::init();
    let settings = cosmic::app::Settings::default()
        .no_main_window(true)
        .transparent(true)
        .exit_on_close(false);
    cosmic::app::run::<Island>(settings, flags)
}

impl Tab {
    /// The section a tab shows; settings is no section and always there.
    fn section(self) -> Option<config::Section> {
        use config::Section;
        match self {
            Tab::Home => Some(Section::Home),
            Tab::Media => Some(Section::Media),
            Tab::Shelf => Some(Section::Shelf),
            Tab::Calendar => Some(Section::Calendar),
            Tab::Tasks => Some(Section::Tasks),
            Tab::Settings => None,
        }
    }

    pub const SECTIONS: [Tab; 5] = [Tab::Home, Tab::Media, Tab::Shelf, Tab::Calendar, Tab::Tasks];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    Home,
    #[default]
    Media,
    Shelf,
    Calendar,
    Tasks,
    Settings,
}

struct OutputEntry {
    output: WlOutput,
    name: Option<String>,
    surface: Option<window::Id>,
}

struct SurfaceState {
    anim: SizeAnimation,
    /// Size whose input zone and blur region the compositor has.
    synced: Option<Size>,
    expanded: bool,
    /// Expanded by a click: stays open when the pointer leaves.
    pinned: bool,
    hovered: bool,
    /// Bumped on every pointer enter/leave, so stale hover timers are ignored.
    hover_generation: u64,
    /// When the current content (compact or expanded) appeared, for its fade-in.
    content_since: Instant,
    /// A file is dragged over the island.
    drag_over: bool,
    /// Natural size of the content, before the island swells under a dragged file.
    measured: Size,
    /// When something was dropped on the island, for its flash.
    dropped_at: Option<Instant>,
}

/// How much the island swells while a file is held over it.
const DRAG_SWELL: Size = Size::new(14.0, 8.0);
/// Push the island gets when it swallows a dropped file, in px/s.
const DROP_PUSH: (f32, f32) = (-240.0, -160.0);
/// How long the accent flash after a drop lasts.
pub const DROP_FLASH: Duration = Duration::from_millis(700);

impl SurfaceState {
    /// Where the pill is heading: the content's size, swollen while a file is held over it.
    fn target(&self) -> Size {
        if self.drag_over && self.expanded {
            Size::new(
                self.measured.width + DRAG_SWELL.width,
                self.measured.height + DRAG_SWELL.height,
            )
        } else {
            self.measured
        }
    }

    /// How strongly the border glows in the accent color, 0–1.
    fn glow(&self, now: Instant) -> f32 {
        if self.drag_over {
            let t = now.saturating_duration_since(self.content_since).as_secs_f32();
            return 0.7 + 0.3 * (t * std::f32::consts::TAU * 0.8).cos();
        }
        match self.dropped_at {
            Some(at) => {
                let t = now.saturating_duration_since(at).as_secs_f32() / DROP_FLASH.as_secs_f32();
                (1.0 - t).clamp(0.0, 1.0).powi(2)
            }
            None => 0.0,
        }
    }
}

pub struct Island {
    core: Core,
    config: IslandConfig,
    config_handle: Option<cosmic_config::Config>,
    time_config: TimeAppletConfig,
    panel: PanelGeometry,
    /// Whether the panel keeps room for the island.
    panel_room: crate::panel::Room,
    locale: Locale,
    now: DateTime<Local>,
    outputs: Vec<OutputEntry>,
    surfaces: HashMap<window::Id, SurfaceState>,
    tab: Tab,
    start_expanded: bool,
    media: Option<MediaState>,
    /// Cover art of the current track: its URL and the loaded image.
    art: Option<(String, Option<cosmic::widget::image::Handle>)>,
    /// Position the user is dragging the progress slider to, in seconds.
    seeking: Option<f32>,
    /// First day of the month shown in the calendar tab.
    calendar_month: chrono::NaiveDate,
    shelves: shelf::Shelves,
    /// The shelf whose name is being edited, with the name typed so far.
    renaming: Option<(usize, String)>,
    share_targets: Vec<ShareTarget>,
    /// Where the shelf is kept across restarts.
    state_handle: Option<cosmic_config::Config>,
    /// Session bus connection serving the daemon's interface.
    dbus: Option<zbus::Connection>,
    /// Width of the compact island, published for the panel applet.
    compact_width: u32,
    /// Levels of the bars next to the playing track.
    bar_levels: crate::modules::audio::Levels,
    /// Thumbnails of shelf files by URI; `None` while loading or when there is none.
    thumbnails: HashMap<String, Option<cosmic::widget::image::Handle>>,
    /// When files landed on the shelf, for their arrival animation.
    landed: HashMap<String, Instant>,
    /// A non-empty shelf whose delete button was pressed once and awaits confirmation.
    confirm_delete: Option<usize>,
    /// The shelf tab shows the devices to send its files to.
    share_page: bool,
    /// LocalSend devices found while the share page is open.
    devices: Vec<crate::modules::localsend::Device>,
    /// How sending goes, by device fingerprint (or KDE Connect id).
    sends: HashMap<String, crate::modules::localsend::SendEvent>,
    /// A snippet whose text was just copied, to say so on its tile.
    copied: Option<(String, Instant)>,
    /// The media tab shows lyrics instead of the cover and title.
    lyrics_view: bool,
    /// Lyrics of the playing track; `None` inside while loading.
    lyrics: Option<(lyrics::Track, Option<lyrics::Lyrics>)>,
    todos: crate::modules::tasks::Todos,
    /// Text of the task being typed.
    todo_input: String,
    /// The task whose text is being edited, with the text so far.
    todo_editing: Option<(u64, String)>,
    /// When tasks were ticked off, for their leaving animation.
    ticked: HashMap<u64, Instant>,
    google: sync::GoogleSync,
    /// Outputs where a fullscreen window is in front.
    covered: crate::modules::fullscreen::Covered,
}

#[derive(Debug, Clone)]
pub enum Message {
    Tick,
    Frame(Instant),
    SyncSurfaces,
    Measured(window::Id, Size),
    PointerEntered(window::Id),
    PointerLeft(window::Id),
    HoverElapsed(window::Id, u64),
    PillPressed(window::Id),
    Escape,
    SelectTab(Tab),
    OutputAdded(WlOutput, Option<String>),
    OutputRemoved(WlOutput),
    Config(IslandConfig),
    TimeConfig(TimeAppletConfig),
    Settings(settings::Message),
    Surface(cosmic::surface::Action),
    Media(Option<MediaState>),
    MediaCommand(media::Command),
    Art(String, Option<std::path::PathBuf>),
    Seek(f32),
    SeekReleased,
    AudioLevels(crate::modules::audio::Levels),
    Thumbnail(String, Option<std::path::PathBuf>),
    CalendarShift(i32),
    DragEntered(window::Id),
    DragLeft(window::Id),
    Dropped(window::Id, Option<Clip>),
    /// Ctrl+V or the paste button: add what is on the clipboard to the shelf.
    Paste,
    Pasted(Option<Clip>),
    /// A shelf tile was clicked: copy a snippet's text, open a file.
    OpenItem(String),
    ShelfRemove(String),
    ShelfClear,
    ShelfSelect(usize),
    ShelfCreate,
    ShelfRenameStart(usize),
    ShelfRenameInput(String),
    ShelfRenameCommit,
    ShelfDelete(usize),
    ShelfDeleteCancel,
    /// A file dragged over a shelf's name switches to that shelf.
    DragOverShelf(usize),
    Share(usize),
    ShareTargets(Vec<ShareTarget>),
    OpenShare,
    CloseShare,
    DeviceFound(crate::modules::localsend::Device),
    SendTo(String),
    SendUpdate(String, crate::modules::localsend::SendEvent),
    ToggleLyrics,
    LyricsLoaded(lyrics::Track, lyrics::Lyrics),
    PanelConfig(PanelKeys),
    Ipc(crate::ipc::Event),
    TodoInput(String),
    TodoAdd,
    TodoToggle(u64, bool),
    TodoRemove(u64),
    TodoEditStart(u64),
    TodoEditInput(String),
    TodoEditCommit,
    /// A ticked task's moment is over: remove it unless it was unticked.
    TodoPurge(u64),
    /// Time to sync the tasks with Google.
    GoogleSync,
    GooglePulled(Result<(String, Vec<(String, String)>), crate::modules::google_tasks::Error>),
    GoogleApplied(Box<sync::Applied>),
    OpenOnlineAccounts,
    Fullscreen(crate::modules::fullscreen::Covered),
}

impl cosmic::Application for Island {
    type Executor = cosmic::executor::Default;
    type Flags = Flags;
    type Message = Message;
    const APP_ID: &'static str = APP_ID;

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(mut core: Core, flags: Flags) -> (Self, Task<Message>) {
        core.set_app_type(cosmic::core::AppType::System);
        // The island manages its own blur region and corners; whole-surface
        // defaults would blur the transparent area around the pill.
        core.set_auto_blur(cosmic::core::Auto::Window.into());
        core.set_auto_corner_radius(cosmic::core::Auto::Window.into());

        let island = Self {
            core,
            config: config::load(APP_ID, config::CONFIG_VERSION),
            config_handle: cosmic_config::Config::new(APP_ID, config::CONFIG_VERSION).ok(),
            time_config: config::load(config::TIME_APPLET_ID, 1),
            panel: PanelGeometry::load(),
            panel_room: PanelKeys::load().room(),
            locale: clock::system_locale(),
            now: Local::now(),
            outputs: Vec::new(),
            surfaces: HashMap::new(),
            tab: flags.expand.unwrap_or_default(),
            start_expanded: flags.expand.is_some(),
            media: None,
            art: None,
            seeking: None,
            calendar_month: crate::modules::calendar::shift_month(Local::now().date_naive(), 0),
            shelves: shelf::Shelves::new(crate::fl!("shelf-default")),
            renaming: None,
            share_targets: Vec::new(),
            state_handle: cosmic_config::Config::new_state(APP_ID, config::CONFIG_VERSION).ok(),
            dbus: None,
            compact_width: 0,
            bar_levels: [0.0; crate::modules::audio::BANDS],
            thumbnails: HashMap::new(),
            landed: HashMap::new(),
            confirm_delete: None,
            share_page: false,
            devices: Vec::new(),
            sends: HashMap::new(),
            lyrics_view: false,
            lyrics: None,
            copied: None,
            todos: Default::default(),
            todo_input: String::new(),
            todo_editing: None,
            ticked: HashMap::new(),
            google: Default::default(),
            covered: Default::default(),
        };
        let mut island = island;
        if flags.expand.is_none() {
            island.tab = island.first_tab();
        }
        island.load_shelf();
        island.load_todos();
        crate::modules::system::start();
        let thumbnails = island.load_thumbnails();
        let sync = island.sync_tasks();
        (island, Task::batch([thumbnails, detect_share_targets(), sync]))
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Tick => {
                self.now = Local::now();
            }
            Message::Frame(now) => {
                return self.sync_surfaces(now);
            }
            Message::SyncSurfaces => {
                return self.sync_surfaces(Instant::now());
            }
            Message::Measured(id, size) => {
                tracing::debug!("island content measures {size:?}");
                if let Some(state) = self.surfaces.get_mut(&id) {
                    state.measured = size;
                    state.anim.go(state.target(), Instant::now());
                    let mut tasks = vec![self.sync_surfaces(Instant::now())];
                    if !self.surfaces[&id].expanded {
                        tasks.push(self.publish_compact_width(size.width));
                    }
                    return Task::batch(tasks);
                }
            }
            Message::PointerEntered(id) => {
                tracing::debug!("pointer entered {id:?}");
                let delay = Duration::from_millis(self.config.hover_delay_ms);
                if let Some(state) = self.surfaces.get_mut(&id) {
                    state.hovered = true;
                    state.hover_generation += 1;
                    if !state.expanded {
                        return delayed(delay, Message::HoverElapsed(id, state.hover_generation));
                    }
                }
            }
            Message::PointerLeft(id) => {
                tracing::debug!("pointer left {id:?}");
                let delay = Duration::from_millis(self.config.collapse_delay_ms);
                if let Some(state) = self.surfaces.get_mut(&id) {
                    state.hovered = false;
                    state.hover_generation += 1;
                    if state.expanded && !state.pinned {
                        return delayed(delay, Message::HoverElapsed(id, state.hover_generation));
                    }
                }
            }
            Message::HoverElapsed(id, generation) => {
                if let Some(state) = self.surfaces.get(&id)
                    && state.hover_generation == generation
                    && !state.pinned
                {
                    return self.set_expanded(id, state.hovered);
                }
            }
            Message::PillPressed(id) => {
                if let Some(state) = self.surfaces.get_mut(&id) {
                    if state.expanded && !state.pinned {
                        // Opened by hovering: a click keeps it open.
                        state.pinned = true;
                    } else {
                        let expand = !state.expanded;
                        state.pinned = expand;
                        return self.set_expanded(id, expand);
                    }
                }
            }
            Message::Escape => {
                let ids: Vec<_> = self
                    .surfaces
                    .iter()
                    .filter(|(_, s)| s.expanded)
                    .map(|(id, _)| *id)
                    .collect();
                return Task::batch(ids.into_iter().map(|id| {
                    if let Some(state) = self.surfaces.get_mut(&id) {
                        state.pinned = false;
                    }
                    self.set_expanded(id, false)
                }));
            }
            Message::SelectTab(tab) => {
                if !self.shows(tab) {
                    return Task::none();
                }
                self.tab = tab;
                match tab {
                    Tab::Shelf => return detect_share_targets(),
                    Tab::Tasks => {
                        return Task::batch([
                            cosmic::widget::text_input::focus(todo::new_task_input_id()),
                            self.sync_tasks(),
                        ]);
                    }
                    _ => {}
                }
            }
            Message::OutputAdded(output, name) => {
                self.outputs.retain(|o| o.output != output);
                self.outputs.push(OutputEntry {
                    output,
                    name,
                    surface: None,
                });
                return self.reconcile_surfaces();
            }
            Message::OutputRemoved(output) => {
                let mut surfaces = Vec::new();
                self.outputs.retain(|o| {
                    if o.output != output {
                        return true;
                    }
                    surfaces.extend(o.surface);
                    false
                });
                return Task::batch(surfaces.into_iter().map(|id| self.destroy_surface(id)));
            }
            Message::Config(config) => {
                let outputs_changed = config.outputs != self.config.outputs
                    || config.hide_in_fullscreen != self.config.hide_in_fullscreen;
                self.config = config;
                if !self.shows(self.tab) {
                    self.tab = self.first_tab();
                }
                if outputs_changed {
                    return self.reconcile_surfaces();
                }
            }
            Message::TimeConfig(config) => {
                self.time_config = config;
            }
            Message::Settings(message) => {
                return self.update_settings(message);
            }
            Message::Surface(action) => {
                return cosmic::task::message(cosmic::Action::Cosmic(
                    cosmic::app::Action::Surface(action),
                ));
            }
            Message::Media(state) => {
                let art_url = state.as_ref().and_then(|s| s.art_url.clone());
                self.media = state;
                let lyrics = self.load_lyrics();
                if art_url.as_deref() != self.art.as_ref().map(|(url, _)| url.as_str()) {
                    self.art = art_url.clone().map(|url| (url, None));
                    if let Some(url) = art_url {
                        return Task::batch([
                            lyrics,
                            cosmic::task::future(async move {
                                let path = media::fetch_art(url.clone()).await;
                                Message::Art(url, path)
                            }),
                        ]);
                    }
                }
                return lyrics;
            }
            Message::ToggleLyrics => {
                self.lyrics_view = !self.lyrics_view;
                return self.load_lyrics();
            }
            Message::LyricsLoaded(track, found) => {
                if let Some((current, slot)) = &mut self.lyrics
                    && *current == track
                {
                    *slot = Some(found);
                }
            }
            Message::Art(url, path) => {
                if let Some((current, handle)) = &mut self.art
                    && *current == url
                {
                    *handle = path.map(cosmic::widget::image::Handle::from_path);
                }
            }
            Message::MediaCommand(command) => {
                if let Some(state) = &self.media {
                    let (player, track_id) = (state.player.clone(), state.track_id.clone());
                    return cosmic::iced::Task::<()>::future(media::command(player, track_id, command)).discard();
                }
            }
            Message::Seek(seconds) => {
                self.seeking = Some(seconds);
            }
            Message::SeekReleased => {
                if let Some(seconds) = self.seeking.take()
                    && let Some(state) = &mut self.media
                {
                    let position = Duration::from_secs_f32(seconds.max(0.0));
                    // Show the new position right away instead of waiting for the player.
                    state.position = position;
                    state.position_at = Instant::now();
                    let (player, track_id) = (state.player.clone(), state.track_id.clone());
                    return cosmic::iced::Task::<()>::future(media::command(
                        player,
                        track_id,
                        media::Command::SetPosition(position),
                    ))
                    .discard();
                }
            }
            Message::DragEntered(_) | Message::Dropped(..) | Message::Paste
                if !self.shows(Tab::Shelf) =>
            {
                // Without the shelf, nothing can be dropped on the island.
            }
            Message::DragEntered(id) => {
                // A file over the island opens the shelf, and the island swells to take it.
                self.tab = Tab::Shelf;
                self.share_page = false;
                let task = self.set_expanded(id, true);
                if let Some(state) = self.surfaces.get_mut(&id) {
                    state.drag_over = true;
                    state.hovered = true;
                    state.hover_generation += 1;
                    state.anim.go(state.target(), Instant::now());
                }
                return task;
            }
            Message::DragLeft(id) => {
                if let Some(state) = self.surfaces.get_mut(&id) {
                    state.drag_over = false;
                    state.anim.go(state.target(), Instant::now());
                }
                return self.update(Message::PointerLeft(id));
            }
            Message::Dropped(id, clip) => {
                let now = Instant::now();
                if let Some(state) = self.surfaces.get_mut(&id) {
                    // The island swallows the file: a short dip back to its size.
                    state.drag_over = false;
                    state.dropped_at = clip.is_some().then_some(now);
                    state.anim.bounce(state.target(), DROP_PUSH, now);
                }
                return self.add_clip(clip);
            }
            Message::Paste => {
                if !self.surfaces.values().any(|s| s.expanded) {
                    return Task::none();
                }
                self.tab = Tab::Shelf;
                self.share_page = false;
                return cosmic::iced::clipboard::read_data::<Clip>()
                    .map(|clip| cosmic::Action::App(Message::Pasted(clip)));
            }
            Message::Pasted(clip) => {
                return self.add_clip(clip);
            }
            Message::OpenItem(uri) => {
                let Some(item) = self.shelves.all_items().find(|i| i.uri == uri).cloned() else {
                    return Task::none();
                };
                if let Some(text) = item.text {
                    self.copied = Some((uri, Instant::now()));
                    return Task::batch([
                        cosmic::iced::clipboard::write(text),
                        delayed(Duration::from_millis(1500), Message::Tick),
                    ]);
                }
                if let Err(err) = std::process::Command::new("xdg-open").arg(&uri).spawn() {
                    tracing::warn!("cannot open {uri}: {err}");
                }
            }
            Message::ShelfRemove(uri) => {
                self.shelves.current_mut().items.retain(|i| i.uri != uri);
                self.save_shelf();
            }
            Message::ShelfSelect(index) => {
                self.share_page = false;
                self.shelves.select(index);
                self.renaming = None;
                self.confirm_delete = None;
                self.save_shelf();
            }
            Message::DragOverShelf(index) => {
                self.shelves.select(index);
            }
            Message::ShelfCreate => {
                let number = self.shelves.list.len() + 1;
                let name = crate::fl!("shelf-new", number = number);
                let index = self.shelves.create(name.clone());
                // Name it right away.
                self.renaming = Some((index, name));
                self.save_shelf();
                return cosmic::widget::text_input::focus(shelf_name_input_id());
            }
            Message::ShelfRenameStart(index) => {
                if let Some(name) = self.shelves.list.get(index).map(|s| s.name.clone()) {
                    self.shelves.select(index);
                    self.renaming = Some((index, name));
                    return cosmic::widget::text_input::focus(shelf_name_input_id());
                }
            }
            Message::ShelfRenameInput(name) => {
                if let Some((_, editing)) = &mut self.renaming {
                    *editing = name;
                }
            }
            Message::ShelfRenameCommit => {
                if let Some((index, name)) = self.renaming.take() {
                    self.shelves.rename(index, &name);
                    self.save_shelf();
                }
            }
            Message::ShelfDelete(index) => {
                // Deleting files asks once more; an empty shelf just goes.
                let empty = self.shelves.list.get(index).is_some_and(|s| s.items.is_empty());
                if !empty && self.confirm_delete != Some(index) {
                    self.confirm_delete = Some(index);
                    return Task::none();
                }
                self.confirm_delete = None;
                self.renaming = None;
                self.shelves.delete(index);
                self.save_shelf();
            }
            Message::ShelfDeleteCancel => {
                self.confirm_delete = None;
            }
            Message::Thumbnail(uri, path) => {
                self.thumbnails
                    .insert(uri, path.map(cosmic::widget::image::Handle::from_path));
            }
            Message::AudioLevels(levels) => {
                self.bar_levels = levels;
            }
            Message::ShelfClear => {
                self.shelves.current_mut().items.clear();
                self.save_shelf();
            }
            Message::Share(index) => {
                if let Some(target) = self.share_targets.get(index) {
                    let ShareTarget::KdeConnect { id, .. } = target;
                    self.sends
                        .insert(id.clone(), crate::modules::localsend::SendEvent::Done);
                    let paths: Vec<_> = self
                        .shelves
                        .current()
                        .items
                        .iter()
                        .filter_map(|i| i.path.clone())
                        .collect();
                    if let Err(err) = target.share(&paths) {
                        tracing::error!("sharing with {} failed: {err}", target.label());
                    }
                }
            }
            Message::OpenShare => {
                self.share_page = true;
                self.devices.clear();
                self.sends.clear();
                return detect_share_targets();
            }
            Message::CloseShare => {
                self.share_page = false;
            }
            Message::DeviceFound(device) => {
                if !self
                    .devices
                    .iter()
                    .any(|d| d.info.fingerprint == device.info.fingerprint)
                {
                    self.devices.push(device);
                }
            }
            Message::SendTo(fingerprint) => {
                let Some(device) = self
                    .devices
                    .iter()
                    .find(|d| d.info.fingerprint == fingerprint)
                    .cloned()
                else {
                    return Task::none();
                };
                let paths: Vec<_> = self
                    .shelves
                    .current()
                    .items
                    .iter()
                    .filter_map(|i| i.path.clone())
                    .collect();
                self.sends.insert(
                    fingerprint.clone(),
                    crate::modules::localsend::SendEvent::Waiting,
                );
                return Task::run(crate::modules::localsend::send(device, paths), move |event| {
                    cosmic::Action::App(Message::SendUpdate(fingerprint.clone(), event))
                });
            }
            Message::SendUpdate(key, event) => {
                self.sends.insert(key, event);
            }
            Message::ShareTargets(targets) => {
                self.share_targets = targets;
            }
            Message::PanelConfig(keys) => {
                self.panel_room = keys.room();
                let panel = PanelGeometry::from_keys(&keys);
                if panel != self.panel {
                    self.panel = panel;
                    // Placement is fixed when a layer surface is created: recreate them all.
                    let ids: Vec<_> = self.outputs.iter_mut().filter_map(|o| o.surface.take()).collect();
                    let destroy = Task::batch(ids.into_iter().map(|id| self.destroy_surface(id)));
                    return destroy.chain(self.reconcile_surfaces());
                }
            }
            Message::Ipc(crate::ipc::Event::Ready(connection)) => {
                self.dbus = Some(connection);
                return self.publish_compact_width(self.compact_width as f32);
            }
            Message::Ipc(crate::ipc::Event::AlreadyRunning) => {
                tracing::info!("the island already runs; exiting");
                std::process::exit(0);
            }
            Message::Ipc(crate::ipc::Event::Toggle) => {
                tracing::debug!("toggle requested over D-Bus");
                let expand = !self.surfaces.values().any(|s| s.expanded);
                let ids: Vec<_> = self.surfaces.keys().copied().collect();
                return Task::batch(ids.into_iter().map(|id| {
                    if let Some(state) = self.surfaces.get_mut(&id) {
                        state.pinned = expand;
                    }
                    self.set_expanded(id, expand)
                }));
            }
            Message::TodoInput(text) => {
                self.todo_input = text;
            }
            Message::TodoAdd => {
                if self.todos.add(&self.todo_input).is_some() {
                    self.todo_input.clear();
                    self.save_todos();
                    return self.sync_tasks();
                }
            }
            Message::TodoToggle(id, done) => {
                self.todos.set_done(id, done);
                self.save_todos();
                let google = self.push_change(id, |id| Change::Complete { id, done });
                if done {
                    // Strike it through and let it leave, then remove it.
                    self.ticked.insert(id, Instant::now());
                    return Task::batch([
                        google,
                        delayed(todo::TICK_ANIMATION, Message::TodoPurge(id)),
                    ]);
                }
                self.ticked.remove(&id);
                return google;
            }
            Message::TodoPurge(id) => {
                self.ticked.remove(&id);
                if self.todos.items.iter().any(|t| t.id == id && t.done) {
                    self.todos.remove(id);
                    self.save_todos();
                }
            }
            Message::TodoRemove(id) => {
                let google = self.push_change(id, |id| Change::Delete { id });
                self.todos.remove(id);
                self.save_todos();
                return google;
            }
            Message::TodoEditStart(id) => {
                if let Some(todo) = self.todos.items.iter().find(|t| t.id == id) {
                    self.todo_editing = Some((id, todo.text.clone()));
                    return cosmic::widget::text_input::focus(todo::edit_task_input_id());
                }
            }
            Message::TodoEditInput(text) => {
                if let Some((_, editing)) = &mut self.todo_editing {
                    *editing = text;
                }
            }
            Message::TodoEditCommit => {
                if let Some((id, text)) = self.todo_editing.take() {
                    let title = text.trim().to_string();
                    let google = self.push_change(id, |id| {
                        if title.is_empty() {
                            Change::Delete { id }
                        } else {
                            Change::Rename { id, title }
                        }
                    });
                    self.todos.edit(id, &text);
                    self.save_todos();
                    return google;
                }
            }
            Message::GoogleSync => return self.sync_tasks(),
            Message::GooglePulled(result) => self.pulled(result),
            Message::GoogleApplied(applied) => return self.applied(*applied),
            Message::OpenOnlineAccounts => {
                // GNOME Settings only runs where it thinks it is in GNOME.
                if let Err(err) = std::process::Command::new("gnome-control-center")
                    .arg("online-accounts")
                    .env("XDG_CURRENT_DESKTOP", "GNOME")
                    .spawn()
                {
                    tracing::warn!("cannot open Online Accounts: {err}");
                }
            }
            Message::Fullscreen(covered) => {
                tracing::debug!("fullscreen windows cover {covered:?}");
                self.covered = covered;
                return self.reconcile_surfaces();
            }
            Message::CalendarShift(delta) => {
                self.calendar_month =
                    crate::modules::calendar::shift_month(self.calendar_month, delta);
            }
        }
        Task::none()
    }

    fn system_theme_update(
        &mut self,
        _keys: &[&'static str],
        _new_theme: &cosmic::cosmic_theme::Theme,
    ) -> Task<Message> {
        // libcosmic drops tasks returned from here for system themes; mark the
        // surfaces dirty and let the sync subscription resend their regions.
        for state in self.surfaces.values_mut() {
            state.synced = None;
        }
        Task::none()
    }

    fn subscription(&self) -> Subscription<Message> {
        self.publish_visibility();
        let expanded = self.surfaces.values().any(|s| s.expanded);
        // Every message redraws the island, so the clock ticks only as often as it changes.
        let tick = if expanded || clock::shows_seconds(&self.clock_format()) {
            Duration::from_secs(1)
        } else {
            Duration::from_secs(60)
        };
        let mut subscriptions = vec![
            Subscription::run_with(tick, clock::ticks).map(|()| Message::Tick),
            cosmic::iced::event::listen_with(|event, status, _| match event {
                cosmic::iced::Event::PlatformSpecific(PlatformSpecific::Wayland(
                    wayland::Event::Output(event, output),
                )) => match event {
                    wayland::OutputEvent::Created(info) => {
                        Some(Message::OutputAdded(output, info.and_then(|i| i.name)))
                    }
                    wayland::OutputEvent::Removed => Some(Message::OutputRemoved(output)),
                    wayland::OutputEvent::InfoUpdate(_) => None,
                },
                cosmic::iced::Event::PlatformSpecific(PlatformSpecific::Wayland(
                    wayland::Event::BlurEnabled,
                )) => Some(Message::SyncSurfaces),
                cosmic::iced::Event::Keyboard(keyboard::Event::KeyPressed {
                    key: keyboard::Key::Named(Named::Escape),
                    ..
                }) => Some(Message::Escape),
                cosmic::iced::Event::Keyboard(keyboard::Event::KeyPressed {
                    key: keyboard::Key::Character(c),
                    modifiers,
                    ..
                }) if modifiers.control()
                    && c.as_str().eq_ignore_ascii_case("v")
                    && status == cosmic::iced::event::Status::Ignored =>
                {
                    Some(Message::Paste)
                }
                _ => None,
            }),
            Subscription::run(media::watch).map(Message::Media),
            Subscription::run(crate::ipc::serve).map(Message::Ipc),
            Subscription::run(crate::modules::fullscreen::watch).map(Message::Fullscreen),
            self.core
                .watch_config::<PanelKeys>(PANEL_CONFIG_ID)
                .map(|update| Message::PanelConfig(update.config)),
            self.core
                .watch_config::<IslandConfig>(APP_ID)
                .map(|update| Message::Config(update.config)),
            self.core
                .watch_config::<TimeAppletConfig>(config::TIME_APPLET_ID)
                .map(|update| Message::TimeConfig(update.config)),
        ];

        let now = Instant::now();
        let receiving = self.surfaces.values().any(|s| s.drag_over);
        let landing = self
            .landed
            .values()
            .any(|at| !geometry::content_transition_done(*at, now));
        if receiving
            || landing
            || self.surfaces.values().any(|s| s.glow(now) > 0.0)
            || self
                .ticked
                .values()
                .any(|at| now.duration_since(*at) < todo::TICK_ANIMATION)
            || self.surfaces.values().any(|s| {
                s.anim.is_animating(now)
                    || !geometry::content_transition_done(s.content_since, now)
            })
        {
            subscriptions.push(window::frames().map(|(_, at)| Message::Frame(at)));
        }
        if self.config.google_tasks {
            // Often while the tasks are on screen, now and then otherwise; opening the tab syncs too.
            let tasks_shown = expanded && self.tab == Tab::Tasks;
            let every = Duration::from_secs(if tasks_shown { 60 } else { 600 });
            subscriptions.push(cosmic::iced::time::every(every).map(|_| Message::GoogleSync));
        }
        let sharing = self.share_page
            && self.tab == Tab::Shelf
            && self.surfaces.values().any(|s| s.expanded);
        if sharing {
            subscriptions.push(
                Subscription::run(crate::modules::localsend::discover).map(Message::DeviceFound),
            );
        }
        let playing = self
            .media
            .as_ref()
            .is_some_and(|m| m.status == media::Status::Playing);
        // Synced lyrics move on several times a second.
        let synced_lyrics = matches!(
            self.lyrics.as_ref().and_then(|(_, l)| l.as_ref()),
            Some(lyrics::Lyrics::Synced(_))
        );
        if playing
            && synced_lyrics
            && self.config.lyrics
            && self.tab == Tab::Media
            && self.surfaces.values().any(|s| s.expanded)
        {
            subscriptions
                .push(cosmic::iced::time::every(Duration::from_millis(250)).map(|_| Message::Tick));
        }
        if playing && self.config.show_bars && !self.surfaces.is_empty() {
            subscriptions
                .push(Subscription::run(crate::modules::audio::levels).map(Message::AudioLevels));
        }
        if self.surfaces.values().any(|s| s.synced.is_none()) {
            subscriptions.push(
                cosmic::iced::time::every(Duration::from_millis(100))
                    .map(|_| Message::SyncSurfaces),
            );
        }

        Subscription::batch(subscriptions)
    }

    fn view(&self) -> Element<'_, Message> {
        unreachable!("the island has no main window")
    }

    fn view_window(&self, id: window::Id) -> Element<'_, Message> {
        self.island_view(id)
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        let on_bg = cosmic::theme::active().cosmic().on_bg_color();
        Some(cosmic::iced::theme::Style {
            background_color: Color::TRANSPARENT,
            text_color: on_bg.into(),
            icon_color: on_bg.into(),
        })
    }
}

impl Island {
    /// Whether the user keeps this tab in the island.
    fn shows(&self, tab: Tab) -> bool {
        tab.section().is_none_or(|s| self.config.shows(s))
    }

    /// The tab to fall back to when the shown one is hidden.
    fn first_tab(&self) -> Tab {
        let preferred = Tab::default();
        if self.shows(preferred) {
            return preferred;
        }
        Tab::SECTIONS
            .into_iter()
            .find(|t| self.shows(*t))
            .unwrap_or(Tab::Settings)
    }

    fn clock_format(&self) -> String {
        self.config.clock_format.clone().unwrap_or_else(|| {
            clock::default_format(
                self.locale,
                self.time_config.military_time,
                self.time_config.show_seconds,
            )
        })
    }

    fn clock_text(&self) -> String {
        clock::format(&self.now, &self.clock_format(), self.locale)
    }

    /// Tells the background workers what is on screen, so they only wake the
    /// island for what it shows.
    fn publish_visibility(&self) {
        let playing = self
            .media
            .as_ref()
            .is_some_and(|m| m.status == media::Status::Playing);
        let lyrics_page = self.lyrics_view && self.config.lyrics;
        let bars_shown = playing
            && self.config.show_bars
            && self.surfaces.values().any(|s| {
                !s.expanded
                    || self.tab == Tab::Home
                    || (self.tab == Tab::Media && !lyrics_page)
            });
        crate::modules::audio::set_shown(bars_shown);
        crate::modules::system::set_detailed(
            self.tab == Tab::Home && self.surfaces.values().any(|s| s.expanded),
        );
    }

    fn set_expanded(&mut self, id: window::Id, expanded: bool) -> Task<Message> {
        let Some(state) = self.surfaces.get_mut(&id) else {
            return Task::none();
        };
        if state.expanded == expanded {
            return Task::none();
        }
        state.expanded = expanded;
        state.content_since = Instant::now();
        if !expanded {
            state.pinned = false;
            self.share_page = false;
            // Reopen on the current month.
            self.calendar_month = crate::modules::calendar::shift_month(self.now.date_naive(), 0);
        }
        // Keyboard focus only while open, so Esc works without stealing focus otherwise.
        set_keyboard_interactivity(
            id,
            if expanded {
                KeyboardInteractivity::OnDemand
            } else {
                KeyboardInteractivity::None
            },
        )
    }

    fn wants_output(&self, entry: &OutputEntry) -> bool {
        use cosmic::applet::cosmic_panel_config::CosmicPanelOuput;
        let named = |names: &[String]| {
            entry
                .name
                .as_ref()
                .is_some_and(|n| names.iter().any(|m| m == n))
        };
        let first = || self.outputs.first().is_some_and(|o| o.output == entry.output);
        if self.config.hide_in_fullscreen {
            // A single island on the active output steps aside for a focused fullscreen window.
            let covered = if self.active_output_only() {
                self.covered.focused
            } else {
                entry.name.as_ref().is_some_and(|n| self.covered.outputs.contains(n))
            };
            if covered {
                return false;
            }
        }
        match &self.config.outputs {
            Outputs::All => true,
            // The active output gets a single surface placed by the compositor.
            Outputs::Active => first(),
            Outputs::Named(names) => named(names),
            Outputs::FollowPanel => match &self.panel.output {
                CosmicPanelOuput::All => true,
                CosmicPanelOuput::Active => first(),
                CosmicPanelOuput::Name(name) => named(std::slice::from_ref(name)),
            },
        }
    }

    fn active_output_only(&self) -> bool {
        use cosmic::applet::cosmic_panel_config::CosmicPanelOuput;
        match self.config.outputs {
            Outputs::Active => true,
            Outputs::FollowPanel => matches!(self.panel.output, CosmicPanelOuput::Active),
            _ => false,
        }
    }

    /// Creates and destroys surfaces so that exactly the wanted outputs have one.
    fn reconcile_surfaces(&mut self) -> Task<Message> {
        let active_only = self.active_output_only();
        let wanted: Vec<bool> = self.outputs.iter().map(|o| self.wants_output(o)).collect();
        let mut tasks = Vec::new();
        for (i, wanted) in wanted.into_iter().enumerate() {
            match (wanted, self.outputs[i].surface) {
                (true, None) => {
                    let id = window::Id::unique();
                    let output = if active_only {
                        IcedOutput::Active
                    } else {
                        IcedOutput::Output(self.outputs[i].output.clone())
                    };
                    self.outputs[i].surface = Some(id);
                    tasks.push(self.create_surface(id, output));
                }
                (false, Some(id)) => {
                    self.outputs[i].surface = None;
                    tasks.push(self.destroy_surface(id));
                }
                _ => {}
            }
        }
        Task::batch(tasks)
    }

    fn create_surface(&mut self, id: window::Id, output: IcedOutput) -> Task<Message> {
        let height = self.panel.compact_height as f32;
        self.surfaces.insert(
            id,
            SurfaceState {
                anim: SizeAnimation::settled(Size::new(height * 3.0, height)),
                synced: None,
                expanded: self.start_expanded,
                pinned: self.start_expanded,
                hovered: false,
                hover_generation: 0,
                content_since: Instant::now(),
                drag_over: false,
                measured: Size::new(height * 3.0, height),
                dropped_at: None,
            },
        );
        let top = self.panel.top as i32;
        cosmic::task::message(cosmic::Action::Cosmic(cosmic::app::Action::Surface(
            app_layer_shell::<Island>(
                |_| cosmic::surface::action::LiveSettings::default(),
                move |_| SctkLayerSurfaceSettings {
                    id,
                    layer: Layer::Overlay,
                    keyboard_interactivity: KeyboardInteractivity::None,
                    input_zone: Some(Vec::new()),
                    anchor: Anchor::TOP,
                    output: output.clone(),
                    namespace: "cosmic-ext-island".into(),
                    margin: IcedMargin {
                        top,
                        ..Default::default()
                    },
                    size: Some((
                        Some(geometry::SURFACE.width as u32),
                        Some(geometry::SURFACE.height as u32),
                    )),
                    // Ignore the panel's exclusive zone, so the island sits over the panel.
                    exclusive_zone: -1,
                    ..Default::default()
                },
                None,
            ),
        )))
    }

    fn destroy_surface(&mut self, id: window::Id) -> Task<Message> {
        self.surfaces.remove(&id);
        cosmic::task::message(cosmic::Action::Cosmic(cosmic::app::Action::Surface(
            destroy_layer_shell(id),
        )))
    }

    /// Sends the input zone and blur region of every surface whose pill changed size.
    fn sync_surfaces(&mut self, now: Instant) -> Task<Message> {
        let theme = cosmic::theme::active();
        let radii = view::Radii::from_theme(theme.cosmic());
        let transparent = theme.transparent;
        let mut tasks = Vec::new();
        for (&id, state) in &mut self.surfaces {
            let size = state.anim.at(now);
            if state.synced == Some(size) {
                continue;
            }
            state.synced = Some(size);
            tasks.push(set_input_zone(id, vec![geometry::pill_bounds(size)]));
            let radius = radii.pill(state.expanded);
            let region = transparent.then(|| geometry::blur_region(size, radius));
            tasks.push(blur(id, region).discard());
        }
        Task::batch(tasks)
    }
}

/// The single shelf of earlier versions, moved into the first shelf.
const LEGACY_SHELF_KEY: &str = "shelf";
const SHELVES_KEY: &str = "shelves";
const CURRENT_SHELF_KEY: &str = "current_shelf";
const TASKS_KEY: &str = "tasks";

pub fn shelf_name_input_id() -> cosmic::widget::Id {
    cosmic::widget::Id::new("shelf-name")
}

impl Island {
    /// The playing track, as lyrics are looked up.
    fn lyrics_track(&self) -> Option<lyrics::Track> {
        let media = self.media.as_ref()?;
        Some(lyrics::Track {
            artist: media.artists.clone(),
            title: media.title.clone(),
            album: media.album.clone(),
            length: media.length,
        })
    }

    /// Starts looking up lyrics of the playing track, if wanted and not done yet.
    pub(super) fn load_lyrics(&mut self) -> Task<Message> {
        let Some(track) = self.lyrics_track() else {
            self.lyrics = None;
            return Task::none();
        };
        if !self.config.lyrics {
            return Task::none();
        }
        if self.lyrics.as_ref().is_some_and(|(t, _)| *t == track) {
            return Task::none();
        }
        self.lyrics = Some((track.clone(), None));
        cosmic::task::future(async move {
            let found = lyrics::fetch(track.clone()).await;
            Message::LyricsLoaded(track, found)
        })
    }

    fn publish_compact_width(&mut self, width: f32) -> Task<Message> {
        let width = width.round() as u32;
        if width == 0 {
            return Task::none();
        }
        self.compact_width = width;
        match self.dbus.clone() {
            Some(connection) => cosmic::iced::Task::<()>::future(
                crate::ipc::set_compact_width(connection, width),
            )
            .discard(),
            None => Task::none(),
        }
    }

    fn load_shelf(&mut self) {
        use cosmic_config::ConfigGet;
        let Some(handle) = &self.state_handle else {
            return;
        };
        let default_name = crate::fl!("shelf-default");
        let stored: shelf::Stored = match handle.get(SHELVES_KEY) {
            Ok(stored) => stored,
            Err(_) => {
                let legacy: Vec<String> = handle.get(LEGACY_SHELF_KEY).unwrap_or_default();
                vec![(default_name.clone(), legacy)]
            }
        };
        let current = handle.get(CURRENT_SHELF_KEY).unwrap_or(0);
        self.shelves = shelf::Shelves::from_stored(stored, current, default_name);
    }

    /// Starts making thumbnails for shelf images that have none yet.
    fn load_thumbnails(&mut self) -> Task<Message> {
        let mut tasks = Vec::new();
        for item in self.shelves.all_items() {
            if self.thumbnails.contains_key(&item.uri) {
                continue;
            }
            self.thumbnails.insert(item.uri.clone(), None);
            let Some(path) = item.path.clone().filter(|p| crate::thumbnail::is_image(p)) else {
                continue;
            };
            let uri = item.uri.clone();
            tasks.push(cosmic::task::future(async move {
                let thumb = tokio::task::spawn_blocking(move || crate::thumbnail::square(&path, 128))
                    .await
                    .ok()
                    .flatten();
                Message::Thumbnail(uri, thumb)
            }));
        }
        Task::batch(tasks)
    }

    /// Adds dropped or pasted things to the shown shelf; text and images become snippet files.
    fn add_clip(&mut self, clip: Option<Clip>) -> Task<Message> {
        let uris = match clip {
            None => return Task::none(),
            Some(Clip::Files(uris)) => uris,
            Some(clip) => match shelf::save_snippet(&clip, Local::now()) {
                Ok(uri) => UriList(vec![uri]),
                Err(err) => {
                    tracing::warn!("cannot keep the pasted content: {err}");
                    return Task::none();
                }
            },
        };
        let current = self.shelves.current;
        let now = Instant::now();
        self.landed.retain(|_, at| now.duration_since(*at) < Duration::from_secs(1));
        for uri in self.shelves.add_to(current, uris) {
            self.landed.insert(uri, now);
        }
        self.save_shelf();
        self.load_thumbnails()
    }

    fn load_todos(&mut self) {
        use cosmic_config::ConfigGet;
        if let Some(handle) = &self.state_handle {
            let stored = handle.get(TASKS_KEY).unwrap_or_default();
            self.todos = crate::modules::tasks::Todos::from_stored(stored);
        }
    }

    fn save_todos(&self) {
        use cosmic_config::ConfigSet;
        if let Some(handle) = &self.state_handle
            && let Err(err) = handle.set(TASKS_KEY, self.todos.items.clone())
        {
            tracing::warn!("failed to save the tasks: {err}");
        }
    }

    fn save_shelf(&self) {
        use cosmic_config::ConfigSet;
        shelf::remove_unused_snippets(&self.shelves);
        let Some(handle) = &self.state_handle else {
            return;
        };
        let result = handle
            .set(SHELVES_KEY, self.shelves.to_stored())
            .and_then(|()| handle.set(CURRENT_SHELF_KEY, self.shelves.current));
        if let Err(err) = result {
            tracing::warn!("failed to save the shelves: {err}");
        }
    }
}

fn detect_share_targets() -> Task<Message> {
    cosmic::task::future(async {
        let targets = tokio::task::spawn_blocking(shelf::detect_share_targets)
            .await
            .unwrap_or_default();
        Message::ShareTargets(targets)
    })
}

fn delayed(delay: Duration, message: Message) -> Task<Message> {
    cosmic::task::future(async move {
        tokio::time::sleep(delay).await;
        message
    })
}

fn set_input_zone<M: 'static>(id: window::Id, zone: Vec<cosmic::iced::Rectangle>) -> Task<M> {
    use cosmic::iced::platform_specific::runtime::{self, wayland};
    cosmic::iced::runtime::task::effect(cosmic::iced::runtime::Action::PlatformSpecific(
        runtime::Action::Wayland(wayland::Action::LayerSurface(
            wayland::layer_surface::Action::InputZone {
                id,
                zone: Some(zone),
            },
        )),
    ))
}
