// SPDX-License-Identifier: GPL-3.0-only

//! The island, as a panel applet: the compact island is the applet itself, so the
//! panel places it among the other applets; the expanded island is a popup that
//! grows out of it.

mod events;
mod home;
mod settings;
mod sync;
mod todo;
mod view;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use chrono::{DateTime, Local, Locale};
use cosmic::app::{Core, Task};
use cosmic::applet::cosmic_panel_config::PanelAnchor;
use cosmic::cosmic_config;
use cosmic::iced::event::{PlatformSpecific, wayland};
use cosmic::iced::keyboard::{self, key::Named};
use cosmic::iced::platform_specific::runtime::wayland::popup::{SctkPopupSettings, SctkPositioner};
use cosmic::iced::platform_specific::shell::commands::{
    blur::blur,
    popup::{destroy_popup, get_popup, set_size},
};
use cosmic::iced::{Color, Rectangle, Size, Subscription, window};
use cosmic::cctk::sctk::reexports::protocols::xdg::shell::client::xdg_positioner::{Anchor, Gravity};
use cosmic::Element;

use crate::applet::APPLET_ID;
use crate::config::{self, APP_ID, IslandConfig, TimeAppletConfig};
use crate::island::geometry::{self, SizeAnimation};
use crate::modules::clock;
use crate::modules::google_tasks::Change;
use crate::modules::lyrics;
use crate::modules::media::{self, MediaState};
use crate::modules::shelf::{self, Clip, ShareTarget, UriList};

/// Options for development and screenshots.
#[derive(Debug, Clone, Default)]
pub struct Flags {
    /// Start expanded and pinned on this tab.
    pub expand: Option<Tab>,
    /// Show the lyrics page of the media tab.
    pub lyrics: bool,
}

pub fn run(flags: Flags) -> cosmic::iced::Result {
    crate::i18n::init();
    cosmic::applet::run::<Island>(flags)
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

struct SurfaceState {
    anim: SizeAnimation,
    /// Size whose blur region the compositor has.
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
    /// Bumped whenever a drag enters or leaves a part of the island, so a leave
    /// followed by an enter (moving from the island onto a shelf's name) is not a leave.
    drag_generation: u64,
    /// Natural size of the content, before the island swells under a dragged file.
    measured: Size,
    /// When something was dropped on the island, for its flash.
    dropped_at: Option<Instant>,
    /// Size of the island when it was last open, which it grows to at once on opening.
    last_expanded: Size,
}

/// How much the island swells while a file is held over it.
const DRAG_SWELL: Size = Size::new(14.0, 8.0);
/// How long a drag may be away from the island before it counts as having left.
const DRAG_LEAVE_GRACE: Duration = Duration::from_millis(150);
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
        if geometry::reduced_motion() {
            // No breathing or flash: the border just takes the accent while a file is over.
            return if self.drag_over { 1.0 } else { 0.0 };
        }
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
    locale: Locale,
    now: DateTime<Local>,
    /// The applet's window, which messages of the island refer to.
    id: window::Id,
    state: SurfaceState,
    /// The popup showing the expanded island, while it is open or closing.
    popup: Option<window::Id>,
    /// Size of the popup's surface; its input region is the whole of it.
    popup_size: Size,
    /// Size of the compact island in the panel.
    compact_size: Size,
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
    /// Why the clipboard history cannot run, when it cannot.
    clipboard_error: Option<String>,
    /// The file of a shelf that is being dragged, and the shelf it came from.
    internal_drag: Option<(usize, String)>,
    /// The shelf whose name is being edited, with the name typed so far.
    renaming: Option<(usize, String)>,
    share_targets: Vec<ShareTarget>,
    /// Where the shelf is kept across restarts.
    state_handle: Option<cosmic_config::Config>,
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
    calendar: events::CalendarSync,
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
    /// The compact island in the panel changed size.
    CompactMeasured(Size),
    CompactEntered,
    CompactLeft,
    CompactPressed,
    /// The compositor closed the popup.
    PopupClosed(window::Id),
    Config(IslandConfig),
    TimeConfig(TimeAppletConfig),
    Settings(settings::Message),
    Surface(cosmic::surface::Action),
    Media(Option<MediaState>),
    MediaCommand(media::Command),
    Art(String, Option<std::path::PathBuf>),
    Seek(f32),
    SeekReleased,
    /// A line of the lyrics was clicked: play from there.
    SeekTo(Duration),
    AudioLevels(crate::modules::audio::Levels),
    Thumbnail(String, Option<std::path::PathBuf>),
    CalendarShift(i32),
    DragEntered(window::Id),
    DragLeft(window::Id),
    /// A file held over the island has not come back since it left.
    DragLeftElapsed(u64),
    /// A file of a shelf is being dragged: its shelf and URI.
    InternalDragStart(usize, String),
    InternalDragEnd,
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
    /// Time to fetch the events from Google Calendar.
    /// Something was copied, or the watcher could not start.
    Clipboard(crate::modules::clipboard::Event),
    CalendarSync,
    CalendarPulled(
        chrono::NaiveDate,
        chrono::NaiveDate,
        Result<(String, Vec<crate::modules::google_calendar::Event>), crate::modules::google_calendar::Error>,
    ),
    CalendarSelect(chrono::NaiveDate),
    EventNew,
    EventTitle(String),
    EventAllDay(bool),
    EventStart(String),
    EventEnd(String),
    EventSubmit,
    EventCancel,
    EventCreated(Result<crate::modules::google_calendar::Event, crate::modules::google_calendar::Error>),
}

impl cosmic::Application for Island {
    type Executor = cosmic::executor::Default;
    type Flags = Flags;
    type Message = Message;
    const APP_ID: &'static str = APPLET_ID;

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, flags: Flags) -> (Self, Task<Message>) {
        let id = core.main_window_id().unwrap_or_else(window::Id::unique);
        let config: IslandConfig = config::load(APP_ID, config::CONFIG_VERSION);
        geometry::set_reduced_motion(config.reduce_motion);
        let (_, thickness) = applet_thickness(&core);
        let compact = Size::new(thickness as f32 * 3.0, thickness as f32);
        let island = Self {
            core,
            config,
            config_handle: cosmic_config::Config::new(APP_ID, config::CONFIG_VERSION).ok(),
            time_config: config::load(config::TIME_APPLET_ID, 1),
            locale: clock::system_locale(),
            now: Local::now(),
            id,
            state: SurfaceState {
                anim: SizeAnimation::settled(compact),
                synced: None,
                expanded: false,
                pinned: false,
                hovered: false,
                hover_generation: 0,
                content_since: Instant::now(),
                drag_over: false,
                drag_generation: 0,
                measured: compact,
                dropped_at: None,
                last_expanded: Size::new(view::EXPANDED_WIDTH + 24.0, 300.0),
            },
            popup: None,
            popup_size: Size::ZERO,
            compact_size: compact,
            tab: flags.expand.unwrap_or_default(),
            start_expanded: flags.expand.is_some(),
            media: None,
            art: None,
            seeking: None,
            calendar_month: crate::modules::calendar::shift_month(Local::now().date_naive(), 0),
            shelves: shelf::Shelves::new(crate::fl!("shelf-default")),
            internal_drag: None,
            clipboard_error: None,
            renaming: None,
            share_targets: Vec::new(),
            state_handle: cosmic_config::Config::new_state(APP_ID, config::CONFIG_VERSION).ok(),
            bar_levels: [0.0; crate::modules::audio::BANDS],
            thumbnails: HashMap::new(),
            landed: HashMap::new(),
            confirm_delete: None,
            share_page: false,
            devices: Vec::new(),
            sends: HashMap::new(),
            lyrics_view: flags.lyrics,
            lyrics: None,
            copied: None,
            todos: Default::default(),
            todo_input: String::new(),
            todo_editing: None,
            ticked: HashMap::new(),
            google: Default::default(),
            calendar: events::CalendarSync::new(Local::now().date_naive()),
        };
        let mut island = island;
        if flags.expand.is_none() {
            island.tab = island.first_tab();
        }
        island.load_shelf();
        if island.config.clipboard_history {
            island.shelves.enable_history(crate::fl!("clipboard"));
        }
        island.load_todos();
        crate::modules::system::start();
        let thumbnails = island.load_thumbnails();
        let sync = island.sync_tasks();
        let calendar = island.sync_calendar();
        (island, Task::batch([thumbnails, detect_share_targets(), sync, calendar]))
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
            Message::CompactMeasured(size) => {
                if size != self.compact_size {
                    self.compact_size = size;
                    if !self.state.expanded && self.popup.is_none() {
                        self.state.measured = size;
                        self.state.anim = SizeAnimation::settled(size);
                    }
                }
                if std::mem::take(&mut self.start_expanded) {
                    self.state.pinned = true;
                    return self.set_expanded(self.id, true);
                }
            }
            Message::CompactEntered => {
                if !self.state.expanded {
                    return self.update(Message::PointerEntered(self.id));
                }
            }
            Message::CompactLeft => {
                if !self.state.expanded {
                    return self.update(Message::PointerLeft(self.id));
                }
            }
            Message::CompactPressed => {
                if !self.state.expanded {
                    return self.update(Message::PillPressed(self.id));
                }
            }
            Message::PopupClosed(popup) => {
                if self.popup == Some(popup) {
                    self.popup = None;
                    self.state.pinned = false;
                    self.state.expanded = false;
                    self.state.anim = SizeAnimation::settled(self.compact_size);
                    self.state.measured = self.compact_size;
                    self.share_page = false;
                }
            }
            Message::Measured(_, size) => {
                tracing::debug!("island content measures {size:?}");
                if self.state.expanded {
                    self.state.last_expanded = size;
                    self.state.measured = size;
                    self.state.anim.go(self.state.target(), Instant::now());
                    return Task::batch([
                        self.resize_popup(),
                        self.sync_surfaces(Instant::now()),
                    ]);
                }
            }
            Message::PointerEntered(_) => {
                let delay = Duration::from_millis(self.config.hover_delay_ms);
                let state = &mut self.state;
                state.hovered = true;
                state.hover_generation += 1;
                if !state.expanded {
                    return delayed(delay, Message::HoverElapsed(self.id, state.hover_generation));
                }
            }
            Message::PointerLeft(_) => {
                let delay = Duration::from_millis(self.config.collapse_delay_ms);
                let state = &mut self.state;
                state.hovered = false;
                state.hover_generation += 1;
                if state.expanded && !state.pinned {
                    return delayed(delay, Message::HoverElapsed(self.id, state.hover_generation));
                }
            }
            Message::HoverElapsed(id, generation) => {
                if self.state.hover_generation == generation && !self.state.pinned {
                    return self.set_expanded(id, self.state.hovered);
                }
            }
            Message::PillPressed(id) => {
                let state = &mut self.state;
                if state.expanded && !state.pinned {
                    // Opened by hovering: a click keeps it open, and the popup
                    // grabs the keyboard, which it needs for typing.
                    state.pinned = true;
                    return self.regrab_popup();
                } else {
                    let expand = !state.expanded;
                    state.pinned = expand;
                    return self.set_expanded(id, expand);
                }
            }
            Message::Escape => {
                if self.state.expanded {
                    self.state.pinned = false;
                    return self.set_expanded(self.id, false);
                }
            }
            Message::SelectTab(tab) => {
                if !self.shows(tab) {
                    return Task::none();
                }
                self.tab = tab;
                match tab {
                    Tab::Shelf => return detect_share_targets(),
                    Tab::Calendar => return self.sync_calendar(),
                    Tab::Media => return self.load_lyrics(),
                    Tab::Tasks => {
                        return Task::batch([
                            cosmic::widget::text_input::focus(todo::new_task_input_id()),
                            self.sync_tasks(),
                        ]);
                    }
                    _ => {}
                }
            }
            Message::Config(config) => {
                geometry::set_reduced_motion(config.reduce_motion);
                self.config = config;
                if !self.shows(self.tab) {
                    self.tab = self.first_tab();
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
            Message::SeekTo(position) => {
                if let Some(state) = &mut self.media {
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
                let state = &mut self.state;
                state.drag_over = true;
                state.drag_generation += 1;
                state.hovered = true;
                state.hover_generation += 1;
                state.anim.go(state.target(), Instant::now());
                return task;
            }
            Message::DragLeft(_) => {
                // Moving onto a shelf's name leaves the island and enters it again at once.
                self.state.drag_generation += 1;
                return delayed(
                    DRAG_LEAVE_GRACE,
                    Message::DragLeftElapsed(self.state.drag_generation),
                );
            }
            Message::DragLeftElapsed(generation) => {
                if generation == self.state.drag_generation && self.state.drag_over {
                    self.state.drag_over = false;
                    self.state.anim.go(self.state.target(), Instant::now());
                    return self.update(Message::PointerLeft(self.id));
                }
            }
            Message::InternalDragStart(shelf, uri) => {
                self.internal_drag = Some((shelf, uri));
            }
            Message::InternalDragEnd => {
                self.internal_drag = None;
            }
            Message::Dropped(_, clip) => {
                let now = Instant::now();
                let state = &mut self.state;
                // The island swallows the file: a short dip back to its size.
                state.drag_over = false;
                state.drag_generation += 1;
                state.dropped_at = clip.is_some().then_some(now);
                state.anim.bounce(state.target(), DROP_PUSH, now);
                if let Some((from, uri)) = self.internal_drag.take() {
                    // A file of one shelf dropped on another moves there.
                    if from == self.shelves.current {
                        return Task::none();
                    }
                    let task = self.add_clip(clip);
                    if let Some(shelf) = self.shelves.list.get_mut(from) {
                        shelf.items.retain(|i| i.uri != uri);
                    }
                    self.save_shelf();
                    return task;
                }
                return self.add_clip(clip);
            }
            Message::Paste => {
                if !self.state.expanded {
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
                // Still over the island, only on a different part of it: nothing to leave.
                self.shelves.select(index);
                self.state.drag_over = true;
                self.state.drag_generation += 1;
                self.state.hovered = true;
                self.state.hover_generation += 1;
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
                if let Some(target) = self.share_targets.get(index).cloned() {
                    use crate::modules::localsend::SendEvent;
                    let ShareTarget::KdeConnect { id, .. } = target.clone();
                    self.sends.insert(id.clone(), SendEvent::Waiting);
                    let paths: Vec<_> = self
                        .shelves
                        .current()
                        .items
                        .iter()
                        .filter_map(|i| i.path.clone())
                        .collect();
                    return cosmic::task::future(async move {
                        let event = match target.share(paths).await {
                            Ok(()) => SendEvent::Done,
                            Err(err) => {
                                tracing::error!("sharing with {} failed: {err}", target.label());
                                SendEvent::Failed(err)
                            }
                        };
                        Message::SendUpdate(id, event)
                    });
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
            Message::Ipc(crate::ipc::Event::Toggle) => {
                tracing::debug!("toggle requested over D-Bus");
                let expand = !self.state.expanded;
                self.state.pinned = expand;
                return self.set_expanded(self.id, expand);
            }
            Message::Ipc(crate::ipc::Event::Ready | crate::ipc::Event::AlreadyRunning) => {}
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
                if done && geometry::reduced_motion() {
                    // Nothing to strike through or slide away.
                    self.todos.remove(id);
                    self.save_todos();
                    return google;
                }
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
                let mut command = if crate::in_flatpak() {
                    let mut command = std::process::Command::new("flatpak-spawn");
                    command.args(["--host", "--env=XDG_CURRENT_DESKTOP=GNOME", "gnome-control-center"]);
                    command
                } else {
                    let mut command = std::process::Command::new("gnome-control-center");
                    command.env("XDG_CURRENT_DESKTOP", "GNOME");
                    command
                };
                if let Err(err) = command.arg("online-accounts").spawn() {
                    tracing::warn!("cannot open Online Accounts: {err}");
                }
            }
            Message::CalendarShift(delta) => {
                self.calendar_month =
                    crate::modules::calendar::shift_month(self.calendar_month, delta);
                // Today when it is in the month, else the first day.
                let today = self.now.date_naive();
                let day = if Island::in_month(self.calendar_month, today) {
                    today
                } else {
                    self.calendar_month
                };
                self.select_day(day);
                return self.sync_calendar();
            }
            Message::Clipboard(event) => return self.clipboard_event(event),
            Message::CalendarSync => return self.sync_calendar(),
            Message::CalendarPulled(from, to, result) => {
                return self.calendar_pulled(from, to, result);
            }
            Message::CalendarSelect(day) => self.select_day(day),
            Message::EventNew => {
                self.calendar.form = Some(events::EventForm::new(self.now));
                return cosmic::widget::text_input::focus(events::event_title_input_id());
            }
            Message::EventTitle(text) => {
                if let Some(form) = &mut self.calendar.form {
                    form.title = text;
                }
            }
            Message::EventAllDay(all_day) => {
                if let Some(form) = &mut self.calendar.form {
                    form.all_day = all_day;
                }
            }
            Message::EventStart(text) => {
                if let Some(form) = &mut self.calendar.form {
                    form.start = text;
                }
            }
            Message::EventEnd(text) => {
                if let Some(form) = &mut self.calendar.form {
                    form.end = text;
                }
            }
            Message::EventSubmit => return self.submit_event(),
            Message::EventCancel => self.calendar.form = None,
            Message::EventCreated(result) => return self.event_created(result),
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
        self.state.synced = None;
        Task::none()
    }

    fn subscription(&self) -> Subscription<Message> {
        self.publish_visibility();
        let expanded = self.state.expanded;
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
                    wayland::Event::Popup(wayland::PopupEvent::Done, _, id),
                )) => Some(Message::PopupClosed(id)),
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
            self.core
                .watch_config::<IslandConfig>(APP_ID)
                .map(|update| Message::Config(update.config)),
            self.core
                .watch_config::<TimeAppletConfig>(config::TIME_APPLET_ID)
                .map(|update| Message::TimeConfig(update.config)),
        ];

        let now = Instant::now();
        let animating = !geometry::reduced_motion();
        let receiving = self.state.drag_over;
        let landing = self
            .landed
            .values()
            .any(|at| !geometry::content_transition_done(*at, now));
        if animating
            && (receiving
            || landing
            || self.state.glow(now) > 0.0
            || self
                .ticked
                .values()
                .any(|at| now.duration_since(*at) < todo::TICK_ANIMATION)
            || self.state.anim.is_animating(now)
            || !geometry::content_transition_done(self.state.content_since, now)
            || (self.popup.is_some() && !self.state.expanded))
        {
            subscriptions.push(window::frames().map(|(_, at)| Message::Frame(at)));
        }
        if self.config.google_tasks {
            // Often while the tasks are on screen, now and then otherwise; opening the tab syncs too.
            let tasks_shown = expanded && self.tab == Tab::Tasks;
            let every = Duration::from_secs(if tasks_shown { 60 } else { 600 });
            subscriptions.push(cosmic::iced::time::every(every).map(|_| Message::GoogleSync));
        }
        if self.config.clipboard_history && self.clipboard_error.is_none() {
            subscriptions
                .push(Subscription::run(crate::modules::clipboard::watch).map(Message::Clipboard));
        }
        if self.config.google_calendar {
            // Often while the calendar is on screen, now and then otherwise; opening the tab syncs too.
            let shown = expanded && self.tab == Tab::Calendar;
            let every = Duration::from_secs(if shown { 60 } else { 900 });
            subscriptions.push(cosmic::iced::time::every(every).map(|_| Message::CalendarSync));
        }
        let sharing = self.share_page
            && self.tab == Tab::Shelf
            && self.state.expanded;
        if sharing {
            subscriptions.push(
                Subscription::run(crate::modules::localsend::discover).map(Message::DeviceFound),
            );
        }
        let playing = self
            .media
            .as_ref()
            .is_some_and(|m| m.status == media::Status::Playing);
        // Synced lyrics change line now and then: wake up for each, and draw
        // frames only while the view scrolls to the new line.
        if playing
            && self.config.lyrics
            && self.tab == Tab::Media
            && self.state.expanded
            && let (Some(media), Some((_, Some(lyrics::Lyrics::Synced(lines))))) =
                (&self.media, &self.lyrics)
        {
            let position = media.position_at(Instant::now());
            let (current, progress) = lyrics::scroll_progress(lines, position);
            if progress < 1.0 && !geometry::reduced_motion() {
                subscriptions.push(window::frames().map(|(_, at)| Message::Frame(at)));
            }
            if let Some(wait) = lyrics::until_next_line(lines, position) {
                subscriptions.push(
                    Subscription::run_with(NextLine { current, wait }, next_line)
                        .map(|()| Message::Tick),
                );
            }
        }
        if playing && self.bars_enabled() {
            subscriptions
                .push(Subscription::run(crate::modules::audio::levels).map(Message::AudioLevels));
        }
        if self.popup.is_some() && self.state.synced.is_none() {
            subscriptions.push(
                cosmic::iced::time::every(Duration::from_millis(100))
                    .map(|_| Message::SyncSurfaces),
            );
        }

        Subscription::batch(subscriptions)
    }

    fn view(&self) -> Element<'_, Message> {
        self.applet_view()
    }

    fn view_window(&self, id: window::Id) -> Element<'_, Message> {
        if self.popup == Some(id) {
            self.island_view(self.id)
        } else {
            self.applet_view()
        }
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
    /// Whether the bars next to the track are drawn: they are animation.
    fn bars_enabled(&self) -> bool {
        self.config.show_bars && !self.config.reduce_motion
    }

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
            && self.bars_enabled()
            && (!self.state.expanded
                || self.tab == Tab::Home
                || (self.tab == Tab::Media && !lyrics_page));
        crate::modules::audio::set_shown(bars_shown);
        // The graphs are kept while the home page can show them; opening it later
        // would otherwise start with empty ones.
        crate::modules::system::set_history_wanted(
            self.config.shows(config::Section::Home)
                && (self.config.shows_card(config::HomeCard::Cpu)
                    || self.config.shows_card(config::HomeCard::Network)),
        );
        crate::modules::system::set_gpu_shown(
            self.tab == Tab::Home && self.state.expanded && self.config.shows_card(config::HomeCard::Gpu),
        );
        crate::modules::system::set_detailed(
            self.tab == Tab::Home && self.state.expanded,
        );
    }

    fn set_expanded(&mut self, _id: window::Id, expanded: bool) -> Task<Message> {
        let now = Instant::now();
        let state = &mut self.state;
        if state.expanded == expanded {
            return Task::none();
        }
        state.expanded = expanded;
        state.content_since = now;
        if !expanded {
            state.pinned = false;
            self.share_page = false;
            // Reopen on the current month.
            self.calendar_month = crate::modules::calendar::shift_month(self.now.date_naive(), 0);
            // Shrink back into the applet; the popup goes when it has.
            state.measured = self.compact_size;
            state.anim.go(self.compact_size, now);
            return self.close_popup_when_done(now);
        }
        // Grow out of the applet.
        state.anim = SizeAnimation::settled(self.compact_size);
        state.synced = None;
        if self.popup.is_some() {
            // Still closing: grow from where it is.
            state.measured = state.last_expanded;
            state.anim.go(state.target(), now);
            return Task::none();
        }
        state.measured = state.last_expanded;
        state.anim.go(state.target(), now);
        let lyrics = self.load_lyrics();
        Task::batch([self.open_popup(), lyrics])
    }

    /// The panel edge the island hangs from.
    fn anchor(&self) -> PanelAnchor {
        self.core.applet.anchor
    }

    /// Size the popup needs for the island at its target size, with room for the spring's overshoot.
    fn popup_wanted(&self) -> Size {
        const ROOM: f32 = 16.0;
        let target = self.state.anim.target();
        Size::new(
            (target.width + ROOM).min(geometry::MAX.width).ceil(),
            (target.height + ROOM).min(geometry::MAX.height).ceil(),
        )
    }

    fn open_popup(&mut self) -> Task<Message> {
        let id = window::Id::unique();
        self.popup = Some(id);
        self.popup_size = self.popup_wanted();
        let (anchor, gravity) = match self.anchor() {
            PanelAnchor::Top => (Anchor::Top, Gravity::Bottom),
            PanelAnchor::Bottom => (Anchor::Bottom, Gravity::Top),
            PanelAnchor::Left => (Anchor::Left, Gravity::Right),
            PanelAnchor::Right => (Anchor::Right, Gravity::Left),
        };
        let (_, thickness) = applet_thickness(&self.core);
        get_popup(SctkPopupSettings {
            parent: self.id,
            id,
            positioner: SctkPositioner {
                size: Some((self.popup_size.width as u32, self.popup_size.height as u32)),
                size_limits: cosmic::iced::Limits::NONE
                    .min_width(1.0)
                    .min_height(1.0)
                    .max_width(geometry::MAX.width)
                    .max_height(geometry::MAX.height),
                anchor_rect: Rectangle {
                    x: 0,
                    y: 0,
                    width: self.compact_size.width.round() as i32,
                    height: if self.core.applet.is_horizontal() {
                        thickness as i32
                    } else {
                        self.compact_size.height.round() as i32
                    },
                },
                anchor,
                gravity,
                // Slide along the panel to stay on the screen.
                constraint_adjustment: 3,
                offset: (0, 0),
                reactive: true,
            },
            parent_size: None,
            // Only a popup the user pinned takes the keyboard (and closes on a click outside).
            grab: self.state.pinned,
            close_with_children: true,
            input_zone: None,
        })
    }

    /// Replaces the popup with one that grabs the keyboard and pointer.
    fn regrab_popup(&mut self) -> Task<Message> {
        let Some(old) = self.popup.take() else {
            return Task::none();
        };
        Task::batch([destroy_popup(old), self.open_popup()])
    }

    /// Resizes the open popup when the island's target size outgrows it.
    fn resize_popup(&mut self) -> Task<Message> {
        let Some(id) = self.popup else {
            return Task::none();
        };
        let wanted = self.popup_wanted();
        if wanted.width <= self.popup_size.width && wanted.height <= self.popup_size.height {
            return Task::none();
        }
        let size = Size::new(
            wanted.width.max(self.popup_size.width),
            wanted.height.max(self.popup_size.height),
        );
        self.popup_size = size;
        self.state.synced = None;
        set_size(id, size.width as u32, size.height as u32)
    }

    /// Takes the popup down once the island has shrunk back into the applet.
    fn close_popup_when_done(&mut self, now: Instant) -> Task<Message> {
        if self.state.expanded || self.state.anim.is_animating(now) {
            return Task::none();
        }
        let Some(id) = self.popup.take() else {
            return Task::none();
        };
        self.popup_size = Size::ZERO;
        destroy_popup(id)
    }

    /// Sends the blur region of the popup when its pill changed size.
    fn sync_surfaces(&mut self, now: Instant) -> Task<Message> {
        let Some(id) = self.popup else {
            return Task::none();
        };
        let theme = cosmic::theme::active();
        let radii = view::Radii::from_theme(theme.cosmic());
        let size = self.state.anim.at(now);
        let task = if self.state.synced == Some(size) {
            Task::none()
        } else {
            self.state.synced = Some(size);
            let radius = radii.pill(self.state.expanded);
            let region = theme
                .transparent
                .then(|| geometry::blur_region(size, radius, self.popup_size, self.anchor()));
            blur(id, region).discard()
        };
        Task::batch([task, self.close_popup_when_done(now)])
    }
}

/// The single shelf of earlier versions, moved into the first shelf.
const LEGACY_SHELF_KEY: &str = "shelf";
const SHELVES_KEY: &str = "shelves";
const CURRENT_SHELF_KEY: &str = "current_shelf";
const HISTORY_KEY: &str = "clipboard_shelf";
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
        // Only looked up where they would show: on the media tab.
        if !self.config.lyrics || !(self.state.expanded && self.tab == Tab::Media) {
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
        self.shelves.history = handle.get(HISTORY_KEY).unwrap_or(false) && self.shelves.list.len() > 1;
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

    /// Keeps a copy on the clipboard shelf.
    fn clipboard_event(&mut self, event: crate::modules::clipboard::Event) -> Task<Message> {
        use crate::modules::clipboard::Event;
        let clip = match event {
            Event::Unavailable(reason) => {
                tracing::warn!("clipboard history unavailable: {reason}");
                self.clipboard_error = Some(reason);
                return Task::none();
            }
            Event::Copied(clip) => clip,
        };
        self.clipboard_error = None;
        let text = match &clip {
            Clip::Text(text) => Some(text.clone()),
            _ => None,
        };
        // A copy already on the shelf only moves up; no new file.
        if let Some(text) = &text
            && self.shelves.bump_history_text(text)
        {
            self.save_shelf();
            return Task::none();
        }
        let uris = match clip {
            Clip::Files(uris) => uris,
            clip => match shelf::save_snippet(&clip, Local::now()) {
                Ok(uri) => UriList(vec![uri]),
                Err(err) => {
                    tracing::warn!("cannot keep the copied content: {err}");
                    return Task::none();
                }
            },
        };
        let now = Instant::now();
        self.landed.retain(|_, at| now.duration_since(*at) < Duration::from_secs(1));
        for uri in self.shelves.add_to_history(uris) {
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
            .and_then(|()| handle.set(CURRENT_SHELF_KEY, self.shelves.current))
            .and_then(|()| handle.set(HISTORY_KEY, self.shelves.history));
        if let Err(err) = result {
            tracing::warn!("failed to save the shelves: {err}");
        }
    }
}

/// Wakes up when the next line of the lyrics starts. Identified by the line
/// being sung, so it is set anew whenever that changes.
struct NextLine {
    current: Option<usize>,
    wait: Duration,
}

impl std::hash::Hash for NextLine {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.current.hash(state);
    }
}

fn next_line(next: &NextLine) -> impl cosmic::iced::futures::Stream<Item = ()> + use<> {
    // A moment late, so the line has started when we look.
    let wait = next.wait + Duration::from_millis(30);
    cosmic::iced::futures::stream::once(async move {
        tokio::time::sleep(wait).await;
    })
}

fn detect_share_targets() -> Task<Message> {
    cosmic::task::future(async { Message::ShareTargets(shelf::detect_share_targets().await) })
}

fn delayed(delay: Duration, message: Message) -> Task<Message> {
    cosmic::task::future(async move {
        tokio::time::sleep(delay).await;
        message
    })
}

/// Thickness of the panel at the applet: its width and height at the panel's cross axis.
fn applet_thickness(core: &Core) -> (u16, u16) {
    let (w, h) = core.applet.suggested_size(true);
    let (pw, ph) = core.applet.suggested_padding(true);
    (w + 2 * pw, h + 2 * ph)
}
