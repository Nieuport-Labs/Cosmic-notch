// SPDX-License-Identifier: GPL-3.0-only

//! Date and time shown in the compact island.

use chrono::{DateTime, Locale, TimeZone};

/// The system locale for date formatting, from `LC_TIME`, `LC_ALL` or `LANG`.
pub fn system_locale() -> Locale {
    ["LC_ALL", "LC_TIME", "LANG"]
        .iter()
        .filter_map(|var| std::env::var(var).ok())
        .find(|v| !v.is_empty() && v != "C" && v != "POSIX")
        .and_then(|v| Locale::try_from(v.split(['.', '@']).next().unwrap_or_default()).ok())
        .unwrap_or(Locale::POSIX)
}

/// Default format of the compact clock, e.g. `so 17. 10. 18:26` in Czech.
pub fn default_format(locale: Locale, military_time: bool, show_seconds: bool) -> String {
    let lang = format!("{locale:?}");
    // Languages that write the day before the month with dots: 17. 10.
    let dotted = ["cs", "sk", "de", "pl", "hu", "sl", "hr", "fi", "da", "nb", "nn", "et"]
        .iter()
        .any(|l| lang.starts_with(l));
    let date = if dotted { "%a %-d. %-m." } else { "%a %b %-d" };
    let time = match (military_time, show_seconds) {
        (true, false) => "%H:%M",
        (true, true) => "%H:%M:%S",
        (false, false) => "%-I:%M %p",
        (false, true) => "%-I:%M:%S %p",
    };
    format!("{date} {time}")
}

/// Languages that write weekday names in lower case, unlike chrono's locale data.
const LOWERCASE_WEEKDAYS: &[&str] = &[
    "cs", "sk", "pl", "sl", "hr", "sr", "bs", "hu", "es", "ca", "fr", "it", "pt", "ro", "nl",
    "da", "nb", "nn", "sv", "fi", "et", "lv", "lt", "ru", "uk", "be", "bg",
];

pub fn format<Tz: TimeZone>(now: &DateTime<Tz>, format: &str, locale: Locale) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let lang = format!("{locale:?}");
    if !LOWERCASE_WEEKDAYS.iter().any(|l| lang.starts_with(l)) {
        return now.format_localized(format, locale).to_string();
    }
    let mut format = format.to_owned();
    for spec in ["%A", "%a"] {
        if format.contains(spec) {
            let name = now.format_localized(spec, locale).to_string().to_lowercase();
            format = format.replace(spec, &name.replace('%', "%%"));
        }
    }
    now.format_localized(&format, locale).to_string()
}

/// Whether a chrono format shows seconds, so the clock needs updating every
/// second rather than every minute.
pub fn shows_seconds(format: &str) -> bool {
    let mut chars = format.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            continue;
        }
        // Padding flags and the colon of `%:z` come before the specifier.
        let spec = chars.by_ref().find(|c| !matches!(c, '-' | '_' | '0' | '^' | '#' | ':'));
        if matches!(spec, Some('S' | 'T' | 'X' | 'r' | 'c' | 's' | '+' | 'f')) {
            return true;
        }
    }
    false
}

/// Ticks when the wall clock passes a whole `period` (a second or a minute),
/// so the clock turns over on time. Checks the time every few seconds at most,
/// so it also catches up right after a suspend.
pub fn ticks(period: &std::time::Duration) -> impl futures_util::Stream<Item = ()> + use<> {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    let period = period.as_millis().max(1);
    let slot = move || {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis())
    };
    futures_util::stream::unfold(slot() / period, move |last| async move {
        loop {
            let now = slot();
            if now / period != last {
                return Some(((), now / period));
            }
            let until_next = (period - now % period) as u64 + 2;
            tokio::time::sleep(Duration::from_millis(until_next.min(5_000))).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_seconds_in_formats() {
        assert!(!shows_seconds("%a %-d. %-m. %H:%M"));
        assert!(shows_seconds("%H:%M:%S"));
        assert!(shows_seconds("%-I:%M:%-S %p"));
        assert!(shows_seconds("%T"));
        assert!(!shows_seconds("100%% %H:%M"));
        assert!(!shows_seconds("%:z %H"));
    }
    use chrono::{FixedOffset, TimeZone};

    fn sample() -> DateTime<FixedOffset> {
        FixedOffset::east_opt(2 * 3600)
            .unwrap()
            .with_ymd_and_hms(2026, 10, 17, 18, 26, 5)
            .unwrap()
    }

    #[test]
    fn czech_24h_matches_design() {
        let fmt = default_format(Locale::cs_CZ, true, false);
        assert_eq!(format(&sample(), &fmt, Locale::cs_CZ), "so 17. 10. 18:26");
    }

    #[test]
    fn english_12h() {
        let fmt = default_format(Locale::en_US, false, false);
        assert_eq!(format(&sample(), &fmt, Locale::en_US), "Sat Oct 17 6:26 PM");
    }

    #[test]
    fn seconds() {
        let fmt = default_format(Locale::cs_CZ, true, true);
        assert_eq!(format(&sample(), &fmt, Locale::cs_CZ), "so 17. 10. 18:26:05");
    }
}
