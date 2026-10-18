// SPDX-License-Identifier: GPL-3.0-only

//! Levels of what the speakers play, for the bars next to the playing track.
//!
//! Records the monitor of the default output only while something plays, and
//! splits each block of samples into a few frequency bands.

use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use cosmic::iced::futures::{SinkExt, Stream, channel::mpsc};

pub const BANDS: usize = 6;
pub type Levels = [f32; BANDS];

const RATE: usize = 16_000;
/// 32 ms of audio per update, about 30 updates per second.
const BLOCK: usize = 512;
/// Band edges in Hz, spaced roughly logarithmically.
const EDGES: [f32; BANDS + 1] = [40.0, 120.0, 300.0, 700.0, 1600.0, 3500.0, 8000.0];

/// Whether any island shows the bars. While none does, levels are still worked
/// out, so the bars come back already settled, but not sent.
static SHOWN: AtomicBool = AtomicBool::new(true);

pub fn set_shown(shown: bool) {
    SHOWN.store(shown, Ordering::Relaxed);
}

/// Changes smaller than this are less than a pixel on the tallest bars.
const UNSEEN_CHANGE: f32 = 0.01;

/// Yields band levels in 0..=1 while it runs.
pub fn levels() -> impl Stream<Item = Levels> {
    cosmic::iced::stream::channel(4, |mut output: mpsc::Sender<Levels>| async move {
        loop {
            // Reading blocks until the sound is played, so it runs on a thread of its own.
            let (tx, mut rx) = tokio::sync::mpsc::channel(4);
            let recorder = std::thread::Builder::new()
                .name("audio-levels".into())
                .spawn(move || record(&tx));
            let recorder = match recorder {
                Ok(handle) => handle,
                Err(err) => {
                    tracing::warn!("audio levels unavailable: {err}");
                    return std::future::pending().await;
                }
            };
            while let Some(levels) = rx.recv().await {
                if output.send(levels).await.is_err() {
                    // The bars are gone; the recorder stops at its next block.
                    return;
                }
            }
            if let Ok(Err(err)) = recorder.join() {
                tracing::warn!("audio levels unavailable: {err}");
            }
            let _ = output.send([0.0; BANDS]).await;
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    })
}

fn record(output: &tokio::sync::mpsc::Sender<Levels>) -> Result<(), String> {
    let mut recording = super::pulse::Recording::monitor(RATE as u32, BLOCK)?;
    let mut analyzer = Analyzer::default();
    let mut raw = [0i16; BLOCK];
    let mut samples = [0.0f32; BLOCK];
    let mut sent: Option<Levels> = None;
    loop {
        recording.read(&mut raw)?;
        for (sample, raw) in samples.iter_mut().zip(raw) {
            *sample = f32::from(raw) / 32768.0;
        }
        let levels = analyzer.push(&samples);
        if !SHOWN.load(Ordering::Relaxed) {
            sent = None;
            if output.is_closed() {
                return Ok(());
            }
            continue;
        }
        // Every update redraws the island; skip the ones nobody could see.
        let unchanged = sent.is_some_and(|last| {
            last.iter()
                .zip(&levels)
                .all(|(a, b)| (a - b).abs() < UNSEEN_CHANGE)
        });
        if unchanged {
            continue;
        }
        sent = Some(levels);
        if output.blocking_send(levels).is_err() {
            return Ok(());
        }
    }
}

/// Hann window and DFT twiddles for a block, worked out once.
struct Tables {
    window: [f32; BLOCK],
    cos: [f32; BLOCK],
    sin: [f32; BLOCK],
}

static TABLES: LazyLock<Tables> = LazyLock::new(|| {
    let angle = |i: usize| std::f32::consts::TAU * i as f32 / BLOCK as f32;
    Tables {
        window: std::array::from_fn(|i| 0.5 - 0.5 * angle(i).cos()),
        cos: std::array::from_fn(|i| angle(i).cos()),
        sin: std::array::from_fn(|i| angle(i).sin()),
    }
});

/// Magnitude of each band in a block of samples (Hann-windowed DFT).
pub fn band_magnitudes(samples: &[f32; BLOCK]) -> Levels {
    let n = BLOCK;
    let bin_hz = RATE as f32 / n as f32;
    let tables = &*TABLES;
    let windowed: [f32; BLOCK] = std::array::from_fn(|i| samples[i] * tables.window[i]);

    let mut levels = [0.0; BANDS];
    for (band, level) in levels.iter_mut().enumerate() {
        let first = (EDGES[band] / bin_hz).ceil().max(1.0) as usize;
        let last = ((EDGES[band + 1] / bin_hz).floor() as usize).min(n / 2);
        let mut power = 0.0;
        for k in first..=last {
            let (mut re, mut im) = (0.0f32, 0.0f32);
            // The angle of sample i in bin k is k·i/n of a turn.
            let mut index = 0;
            for s in &windowed {
                re += s * tables.cos[index];
                im -= s * tables.sin[index];
                index = (index + k) % n;
            }
            power += re * re + im * im;
        }
        *level = (power / (last + 1 - first).max(1) as f32).sqrt() / (n as f32 / 4.0);
    }
    levels
}

/// Turns band magnitudes into lively, self-scaling bar levels.
///
/// Each band is measured against its own running average, so the bars follow
/// beats and changes rather than sitting at the top through loud music.
#[derive(Debug, Clone)]
pub struct Analyzer {
    averages: Levels,
    peaks: Levels,
    levels: Levels,
}

impl Default for Analyzer {
    fn default() -> Self {
        Self {
            averages: [0.0; BANDS],
            peaks: [1e-3; BANDS],
            levels: [0.0; BANDS],
        }
    }
}

impl Analyzer {
    pub fn push(&mut self, samples: &[f32; BLOCK]) -> Levels {
        let magnitudes = band_magnitudes(samples);
        for (((m, average), peak), level) in magnitudes
            .into_iter()
            .zip(&mut self.averages)
            .zip(&mut self.peaks)
            .zip(&mut self.levels)
        {
            // About a second of history.
            *average += (m - *average) * 0.04;
            // Keep headroom above the average, so steady sound sits low and beats jump.
            *peak = (*peak * 0.995).max(m).max(*average * 1.8).max(1e-3);
            let floor = *average * 0.7;
            let target = ((m - floor) / (*peak - floor).max(1e-3))
                .clamp(0.0, 1.0)
                .powf(0.8);
            // Rise fast, fall slowly.
            let k = if target > *level { 0.65 } else { 0.2 };
            *level += (target - *level) * k;
        }
        self.levels
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(hz: f32) -> [f32; BLOCK] {
        std::array::from_fn(|i| (std::f32::consts::TAU * hz * i as f32 / RATE as f32).sin() * 0.5)
    }

    fn loudest(levels: Levels) -> usize {
        (0..BANDS)
            .max_by(|&a, &b| levels[a].total_cmp(&levels[b]))
            .unwrap()
    }

    #[test]
    fn tones_land_in_their_band() {
        assert_eq!(loudest(band_magnitudes(&sine(80.0))), 0);
        assert_eq!(loudest(band_magnitudes(&sine(1000.0))), 3);
        assert_eq!(loudest(band_magnitudes(&sine(5000.0))), 5);
    }

    #[test]
    fn silence_is_flat() {
        let mut analyzer = Analyzer::default();
        let levels = analyzer.push(&[0.0; BLOCK]);
        assert!(levels.iter().all(|&l| l == 0.0));
    }

    #[test]
    fn steady_sound_settles_below_the_top() {
        let mut analyzer = Analyzer::default();
        let mut level = 0.0;
        for _ in 0..200 {
            level = analyzer.push(&sine(1000.0))[3];
        }
        assert!(level < 0.6, "{level}");
    }

    #[test]
    fn levels_rise_with_sound_and_decay_after() {
        let mut analyzer = Analyzer::default();
        let loud = analyzer.push(&sine(1000.0))[3];
        assert!(loud > 0.5, "{loud}");
        let mut quiet = loud;
        for _ in 0..10 {
            quiet = analyzer.push(&[0.0; BLOCK])[3];
        }
        assert!(quiet < 0.2, "{quiet}");
    }
}
