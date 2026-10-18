// SPDX-License-Identifier: GPL-3.0-only

//! What is playing, from MPRIS players on the session bus.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use cosmic::iced::futures::{SinkExt, Stream, StreamExt, channel::mpsc};
use mpris2_zbus::enumerator::{Enumerator, Event as EnumeratorEvent};
use mpris2_zbus::player::{PlaybackStatus, Player};
use tokio::sync::OnceCell;
use zbus::Connection;
use zbus::names::OwnedBusName;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Playing,
    Paused,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MediaState {
    /// Bus name of the player, e.g. `org.mpris.MediaPlayer2.spotify`.
    pub player: String,
    pub title: String,
    pub artists: String,
    pub album: Option<String>,
    pub art_url: Option<String>,
    pub track_id: Option<String>,
    pub length: Option<Duration>,
    /// Position when the state was read, at `position_at`.
    pub position: Duration,
    pub position_at: Instant,
    pub status: Status,
    pub can_go_next: bool,
    pub can_go_previous: bool,
    pub can_seek: bool,
}

impl MediaState {
    /// Position now, extrapolated while playing.
    pub fn position_at(&self, now: Instant) -> Duration {
        let position = match self.status {
            Status::Playing => self.position + now.saturating_duration_since(self.position_at),
            Status::Paused => self.position,
        };
        self.length.map_or(position, |l| position.min(l))
    }

    /// "Artist – Title", or whichever of the two is known.
    pub fn label(&self) -> String {
        match (self.artists.is_empty(), self.title.is_empty()) {
            (false, false) => format!("{} – {}", self.artists, self.title),
            (true, _) => self.title.clone(),
            (false, true) => self.artists.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Command {
    PlayPause,
    Next,
    Previous,
    SetPosition(Duration),
}

/// Formats a duration as `m:ss`, or `h:mm:ss` from an hour on.
pub fn format_duration(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

static CONNECTION: OnceCell<Connection> = OnceCell::const_new();

/// The island's session bus connection, shared by everything that calls out.
pub async fn connection() -> zbus::Result<&'static Connection> {
    CONNECTION.get_or_try_init(Connection::session).await
}

/// Watches all players and yields the one to show whenever it changes.
pub fn watch() -> impl Stream<Item = Option<MediaState>> {
    cosmic::iced::stream::channel(16, |mut output: mpsc::Sender<Option<MediaState>>| async move {
        loop {
            if let Err(err) = run(&mut output).await {
                tracing::warn!("MPRIS watcher stopped: {err}");
            }
            let _ = output.send(None).await;
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    })
}

struct Tracked {
    player: Player,
    /// When the player was last seen starting to play.
    started_playing: Option<Instant>,
    last_status: Option<PlaybackStatus>,
    watcher: tokio::task::JoinHandle<()>,
}

impl Drop for Tracked {
    fn drop(&mut self) {
        self.watcher.abort();
    }
}

async fn run(output: &mut mpsc::Sender<Option<MediaState>>) -> zbus::Result<()> {
    let conn = connection().await?;
    let enumerator = Enumerator::new(conn).await?;
    let mut changes = enumerator.receive_changes().await?;
    let (changed_tx, mut changed_rx) = tokio::sync::mpsc::unbounded_channel::<()>();

    let mut players: HashMap<String, Tracked> = HashMap::new();
    for name in enumerator.players().await? {
        track(conn, name, &changed_tx, &mut players).await;
    }

    let mut resync = tokio::time::interval(Duration::from_secs(2));
    let mut last_sent: Option<Option<MediaState>> = None;
    loop {
        tokio::select! {
            Some(event) = changes.next() => match event? {
                EnumeratorEvent::Add(name) => track(conn, name, &changed_tx, &mut players).await,
                EnumeratorEvent::Remove(name) => {
                    players.remove(name.as_str());
                }
            },
            Some(()) = changed_rx.recv() => {}
            _ = resync.tick() => {
                // Only a playing track drifts or learns its length late; paused
                // players signal what changes.
                let playing = last_sent
                    .as_ref()
                    .is_some_and(|s| s.as_ref().is_some_and(|m| m.status == Status::Playing));
                if !playing {
                    continue;
                }
            }
        }

        let state = current(&mut players).await;
        // Positions are extrapolated by the UI; resend only on real changes or drift.
        if last_sent.as_ref().is_none_or(|last| differs(last, &state)) {
            last_sent = Some(state.clone());
            if output.send(state).await.is_err() {
                return Ok(());
            }
        }
    }
}

fn differs(last: &Option<MediaState>, new: &Option<MediaState>) -> bool {
    match (last, new) {
        (Some(a), Some(b)) => {
            let now = Instant::now();
            let drift = a.position_at(now).abs_diff(b.position_at(now));
            MediaState {
                position: Duration::ZERO,
                position_at: now,
                ..a.clone()
            } != MediaState {
                position: Duration::ZERO,
                position_at: now,
                ..b.clone()
            } || drift > Duration::from_millis(1500)
        }
        (None, None) => false,
        _ => true,
    }
}

async fn track(
    conn: &Connection,
    name: OwnedBusName,
    changed: &tokio::sync::mpsc::UnboundedSender<()>,
    players: &mut HashMap<String, Tracked>,
) {
    let Ok(player) = Player::new(conn, name.clone()).await else {
        return;
    };
    let watcher = {
        let player = player.clone();
        let changed = changed.clone();
        tokio::spawn(async move {
            let status = player.receive_playback_status_changed().await.map(|_| ());
            let metadata = player.receive_metadata_changed().await.map(|_| ());
            let seeked = player.receive_seeked().await.ok().map(|s| s.map(|_| ()));
            let mut events = cosmic::iced::futures::stream::select_all([
                status.boxed(),
                metadata.boxed(),
            ]);
            if let Some(seeked) = seeked {
                events.push(seeked.boxed());
            }
            while events.next().await.is_some() {
                if changed.send(()).is_err() {
                    break;
                }
            }
        })
    };
    players.insert(
        name.to_string(),
        Tracked {
            player,
            started_playing: None,
            last_status: None,
            watcher,
        },
    );
    let _ = changed.send(());
}

/// Picks the player to show: the one that most recently started playing, or
/// failing that, the most recent one that is paused.
async fn current(players: &mut HashMap<String, Tracked>) -> Option<MediaState> {
    let now = Instant::now();
    for tracked in players.values_mut() {
        let status = tracked.player.playback_status().await.ok();
        if status == Some(PlaybackStatus::Playing)
            && tracked.last_status != Some(PlaybackStatus::Playing)
        {
            tracked.started_playing = Some(now);
        }
        tracked.last_status = status;
    }

    let pick = |wanted: PlaybackStatus| {
        players
            .iter()
            .filter(|(_, t)| t.last_status == Some(wanted))
            .max_by_key(|(_, t)| t.started_playing)
            .map(|(name, _)| name.clone())
    };
    let name = pick(PlaybackStatus::Playing).or_else(|| pick(PlaybackStatus::Paused))?;
    let tracked = players.get(&name)?;
    read_state(&name, tracked).await
}

async fn read_state(name: &str, tracked: &Tracked) -> Option<MediaState> {
    let player = &tracked.player;
    let metadata = player.metadata().await.ok()?;
    let title = metadata.title().unwrap_or_default();
    let artists = metadata.artists().unwrap_or_default().join(", ");
    if title.is_empty() && artists.is_empty() {
        return None;
    }
    let to_std = |d: jiff::SignedDuration| Duration::from_micros(d.as_micros().max(0) as u64);
    Some(MediaState {
        player: name.to_owned(),
        title,
        artists,
        album: metadata.album().filter(|a| !a.is_empty()),
        art_url: metadata.art_url().filter(|u| !u.is_empty()),
        track_id: metadata.track_id().map(|p| p.to_string()),
        length: match metadata.length().map(to_std).filter(|l| !l.is_zero()) {
            Some(length) => Some(length),
            // Firefox can learn the length after announcing the track without
            // signalling it, so the proxy's cached copy may lack it: ask the player.
            None => fetch_length(name).await,
        },
        position: fetch_position(name).await.unwrap_or_default(),
        position_at: Instant::now(),
        status: if tracked.last_status == Some(PlaybackStatus::Playing) {
            Status::Playing
        } else {
            Status::Paused
        },
        can_go_next: player.can_go_next().await.unwrap_or(false),
        can_go_previous: player.can_go_previous().await.unwrap_or(false),
        can_seek: player.can_seek().await.unwrap_or(false),
    })
}

/// The track length straight from the player, bypassing the proxy's cache.
async fn fetch_length(player: &str) -> Option<Duration> {
    let conn = connection().await.ok()?;
    let reply = conn
        .call_method(
            Some(player),
            "/org/mpris/MediaPlayer2",
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &("org.mpris.MediaPlayer2.Player", "Metadata"),
        )
        .await
        .ok()?;
    let value: zbus::zvariant::OwnedValue = reply.body().deserialize().ok()?;
    let metadata: HashMap<String, zbus::zvariant::OwnedValue> = value.try_into().ok()?;
    let length = metadata.get("mpris:length")?;
    let micros = i64::try_from(length)
        .ok()
        .or_else(|| u64::try_from(length).ok().map(|u| u as i64))?;
    (micros > 0).then(|| Duration::from_micros(micros as u64))
}

/// Reads `Position` straight from the player: players do not signal changes to
/// it, so the proxy's cached value is stale.
pub async fn fetch_position(player: &str) -> Option<Duration> {
    let conn = connection().await.ok()?;
    let reply = conn
        .call_method(
            Some(player),
            "/org/mpris/MediaPlayer2",
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &("org.mpris.MediaPlayer2.Player", "Position"),
        )
        .await
        .ok()?;
    let value: zbus::zvariant::OwnedValue = reply.body().deserialize().ok()?;
    let micros = i64::try_from(value).ok()?;
    Some(Duration::from_micros(micros.max(0) as u64))
}

pub async fn command(player: String, track_id: Option<String>, command: Command) {
    let result: zbus::Result<()> = async {
        let conn = connection().await?;
        let name = OwnedBusName::try_from(player)?;
        if let Command::SetPosition(position) = command {
            // Called directly: the bindings want a TrackId, which cannot be built from a path.
            let Some(track_id) = track_id else {
                return Ok(());
            };
            let path = zbus::zvariant::ObjectPath::try_from(track_id.as_str())?;
            conn.call_method(
                Some(name),
                "/org/mpris/MediaPlayer2",
                Some("org.mpris.MediaPlayer2.Player"),
                "SetPosition",
                &(path, position.as_micros() as i64),
            )
            .await?;
            return Ok(());
        }
        let player = Player::new(conn, name)
            .await
            .map_err(|e| zbus::Error::Failure(e.to_string()))?;
        match command {
            Command::PlayPause => player.play_pause().await,
            Command::Next => player.next().await,
            Command::Previous => player.previous().await,
            Command::SetPosition(_) => Ok(()),
        }
    }
    .await;
    if let Err(err) = result {
        tracing::warn!("media command failed: {err}");
    }
}

/// Resolves cover art to a square local image, downloading remote art into
/// the cache. Video thumbnails are 16:9, so art is center-cropped.
pub async fn fetch_art(url: String) -> Option<PathBuf> {
    let source = if let Some(path) = url.strip_prefix("file://") {
        PathBuf::from(percent_decode(path))
    } else if url.starts_with("http://") || url.starts_with("https://") {
        download(&url).await?
    } else {
        return None;
    };
    tokio::task::spawn_blocking(move || crate::thumbnail::square(&source, 256))
        .await
        .ok()
        .flatten()
}

async fn download(url: &str) -> Option<PathBuf> {
    use crate::thumbnail::{cache_dir, stable_hash};
    let dir = cache_dir()?.join("art");
    let path = dir.join(format!("{:016x}", stable_hash(url)));
    if tokio::fs::try_exists(&path).await.unwrap_or(false) {
        return Some(path);
    }
    let bytes = async {
        let response = reqwest::get(url).await?.error_for_status()?;
        response.bytes().await
    }
    .await
    .inspect_err(|err| tracing::debug!("cover art {url}: {err}"))
    .ok()?;
    tokio::fs::create_dir_all(&dir).await.ok()?;
    tokio::fs::write(&path, &bytes).await.ok()?;
    Some(path)
}

pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(hex) = s.get(i + 1..i + 3)
            && let Ok(b) = u8::from_str_radix(hex, 16)
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(status: Status) -> MediaState {
        MediaState {
            player: "org.mpris.MediaPlayer2.test".into(),
            title: "Wait and Bleed".into(),
            artists: "Slipknot".into(),
            album: None,
            art_url: None,
            track_id: None,
            length: Some(Duration::from_secs(150)),
            position: Duration::from_secs(40),
            position_at: Instant::now(),
            status,
            can_go_next: true,
            can_go_previous: true,
            can_seek: true,
        }
    }

    #[test]
    fn label_joins_artist_and_title() {
        assert_eq!(state(Status::Playing).label(), "Slipknot – Wait and Bleed");
    }

    #[test]
    fn position_advances_only_while_playing() {
        let (playing, paused) = (state(Status::Playing), state(Status::Paused));
        let later = Instant::now() + Duration::from_secs(10);
        assert!(playing.position_at(later) >= Duration::from_secs(50));
        assert_eq!(paused.position_at(later), Duration::from_secs(40));
    }

    #[test]
    fn position_stops_at_length() {
        let later = Instant::now() + Duration::from_secs(1000);
        assert_eq!(state(Status::Playing).position_at(later), Duration::from_secs(150));
    }

    #[test]
    fn durations() {
        assert_eq!(format_duration(Duration::from_secs(40)), "0:40");
        assert_eq!(format_duration(Duration::from_secs(286)), "4:46");
        assert_eq!(format_duration(Duration::from_secs(3725)), "1:02:05");
    }

    #[test]
    fn decodes_file_urls() {
        assert_eq!(percent_decode("/tmp/My%20Song%C3%A9.jpg"), "/tmp/My Songé.jpg");
        assert_eq!(percent_decode("/tmp/100%"), "/tmp/100%");
    }

    #[test]
    fn small_drift_is_not_a_change() {
        let a = state(Status::Playing);
        let mut b = a.clone();
        b.position += Duration::from_millis(300);
        assert!(!differs(&Some(a.clone()), &Some(b)));
        let mut c = a.clone();
        c.title = "Other".into();
        assert!(differs(&Some(a), &Some(c)));
    }

    /// Needs a player named `org.mpris.MediaPlayer2.mock` on the session bus
    /// whose track id is `/test/track/0` or `/test/track/1`.
    #[tokio::test]
    #[ignore]
    async fn controls_mock_player() {
        let name = "org.mpris.MediaPlayer2.mock".to_owned();
        let conn = connection().await.unwrap();
        let player = Player::new(conn, name.clone().try_into().unwrap()).await.unwrap();
        let before = player.playback_status().await.unwrap();
        command(name.clone(), None, Command::PlayPause).await;
        assert_ne!(player.playback_status().await.unwrap(), before);
        command(name.clone(), None, Command::PlayPause).await;

        let track = player.metadata().await.unwrap().track_id().unwrap().to_string();
        command(name, Some(track), Command::SetPosition(Duration::from_secs(100))).await;
        let position = fetch_position("org.mpris.MediaPlayer2.mock").await.unwrap();
        assert!((99..=101).contains(&position.as_secs()), "{position:?}");
    }
}
