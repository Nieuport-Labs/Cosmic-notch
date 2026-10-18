// SPDX-License-Identifier: GPL-3.0-only

//! Song lyrics from LRCLIB (<https://lrclib.net>), synced to the playing
//! position when LRCLIB has timed lyrics.
//!
//! Looking lyrics up sends the artist, title, album and length of the track to
//! lrclib.net; the settings page can turn it off. Answers are cached on disk.

use std::time::Duration;

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lyrics {
    /// Lines with the time each starts.
    Synced(Vec<(Duration, String)>),
    Plain(Vec<String>),
    /// LRCLIB knows the track but it has no lyrics.
    Instrumental,
    NotFound,
}

/// Identifies a track for looking up and caching its lyrics.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Track {
    pub artist: String,
    pub title: String,
    pub album: Option<String>,
    pub length: Option<Duration>,
}

impl Track {
    fn cache_key(&self) -> String {
        format!("{}\u{1f}{}", self.artist.to_lowercase(), self.title.to_lowercase())
    }
}

/// Parses LRC: `[mm:ss.xx] text` lines, possibly with several time tags.
pub fn parse_lrc(lrc: &str) -> Vec<(Duration, String)> {
    let mut lines = Vec::new();
    for line in lrc.lines() {
        let mut rest = line.trim();
        let mut times = Vec::new();
        while let Some(tag) = rest.strip_prefix('[') {
            let Some(end) = tag.find(']') else {
                break;
            };
            if let Some(time) = parse_time(&tag[..end]) {
                times.push(time);
            }
            rest = &tag[end + 1..];
        }
        let text = rest.trim().to_owned();
        lines.extend(times.into_iter().map(|t| (t, text.clone())));
    }
    lines.sort_by_key(|(t, _)| *t);
    lines
}

fn parse_time(tag: &str) -> Option<Duration> {
    let (minutes, seconds) = tag.split_once(':')?;
    let minutes: u64 = minutes.trim().parse().ok()?;
    let seconds: f64 = seconds.trim().parse().ok()?;
    (seconds >= 0.0).then(|| Duration::from_secs(minutes * 60) + Duration::from_secs_f64(seconds))
}

/// Index of the line being sung at `position`, if one has started.
pub fn current_line(lines: &[(Duration, String)], position: Duration) -> Option<usize> {
    lines.iter().rposition(|(start, _)| *start <= position)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Record {
    #[serde(default)]
    instrumental: bool,
    plain_lyrics: Option<String>,
    synced_lyrics: Option<String>,
}

impl Record {
    fn into_lyrics(self) -> Lyrics {
        if let Some(synced) = self.synced_lyrics.filter(|s| !s.trim().is_empty()) {
            let lines = parse_lrc(&synced);
            if !lines.is_empty() {
                return Lyrics::Synced(lines);
            }
        }
        if let Some(plain) = self.plain_lyrics.filter(|s| !s.trim().is_empty()) {
            return Lyrics::Plain(plain.lines().map(str::to_owned).collect());
        }
        if self.instrumental {
            Lyrics::Instrumental
        } else {
            Lyrics::NotFound
        }
    }
}

fn client() -> Option<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(concat!(
            "cosmic-ext-island/",
            env!("CARGO_PKG_VERSION"),
            " (https://github.com/Nieuport-Labs/Cosmic-notch)"
        ))
        .timeout(Duration::from_secs(10))
        .build()
        .ok()
}

/// Looks lyrics up, from the cache when they were found before.
pub async fn fetch(track: Track) -> Lyrics {
    let cache = crate::thumbnail::cache_dir().map(|dir| {
        dir.join("lyrics").join(format!(
            "{:016x}.json",
            crate::thumbnail::stable_hash(&track.cache_key())
        ))
    });
    if let Some(path) = &cache
        && let Ok(bytes) = tokio::fs::read(path).await
        && let Ok(record) = serde_json::from_slice::<Record>(&bytes)
    {
        return record.into_lyrics();
    }

    let Some(client) = client() else {
        return Lyrics::NotFound;
    };
    let mut query = vec![
        ("artist_name", track.artist.clone()),
        ("track_name", track.title.clone()),
    ];
    if let Some(album) = &track.album {
        query.push(("album_name", album.clone()));
    }
    if let Some(length) = track.length {
        query.push(("duration", length.as_secs().to_string()));
    }

    // Exact match first; a search tolerates differing albums and lengths.
    let exact = async {
        let response = client
            .get("https://lrclib.net/api/get")
            .query(&query)
            .send()
            .await
            .ok()?;
        if !response.status().is_success() {
            return None;
        }
        response.bytes().await.ok()
    }
    .await;
    let bytes = match exact {
        Some(bytes) => Some(bytes.to_vec()),
        None => async {
            let response = client
                .get("https://lrclib.net/api/search")
                .query(&query[..2])
                .send()
                .await
                .ok()?;
            let records: Vec<serde_json::Value> = response.json().await.ok()?;
            // Prefer a result with synced lyrics.
            let best = records
                .iter()
                .find(|r| r["syncedLyrics"].is_string())
                .or_else(|| records.first())?;
            serde_json::to_vec(best).ok()
        }
        .await,
    };

    let Some(bytes) = bytes else {
        return Lyrics::NotFound;
    };
    let Ok(record) = serde_json::from_slice::<Record>(&bytes) else {
        return Lyrics::NotFound;
    };
    if let Some(path) = cache {
        if let Some(dir) = path.parent() {
            let _ = tokio::fs::create_dir_all(dir).await;
        }
        let _ = tokio::fs::write(&path, &bytes).await;
    }
    record.into_lyrics()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lrc_lines() {
        let lines = parse_lrc("[00:12.50] First line\n[ar:Someone]\n[01:02.00]Second\n\n[00:05.00]");
        assert_eq!(
            lines,
            [
                (Duration::from_secs(5), String::new()),
                (Duration::from_millis(12_500), "First line".to_owned()),
                (Duration::from_secs(62), "Second".to_owned()),
            ]
        );
    }

    #[test]
    fn repeated_lines_get_every_time() {
        let lines = parse_lrc("[00:10.00][00:30.00]Chorus");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1].0, Duration::from_secs(30));
    }

    #[test]
    fn finds_the_current_line() {
        let lines = parse_lrc("[00:10.00]a\n[00:20.00]b\n[00:30.00]c");
        assert_eq!(current_line(&lines, Duration::from_secs(5)), None);
        assert_eq!(current_line(&lines, Duration::from_secs(20)), Some(1));
        assert_eq!(current_line(&lines, Duration::from_secs(99)), Some(2));
    }

    #[test]
    fn prefers_synced_lyrics() {
        let record: Record = serde_json::from_str(
            r#"{"instrumental":false,"plainLyrics":"a\nb","syncedLyrics":"[00:01.00]a"}"#,
        )
        .unwrap();
        assert!(matches!(record.into_lyrics(), Lyrics::Synced(_)));
        let record: Record =
            serde_json::from_str(r#"{"instrumental":true,"plainLyrics":null,"syncedLyrics":null}"#)
                .unwrap();
        assert_eq!(record.into_lyrics(), Lyrics::Instrumental);
    }
}
