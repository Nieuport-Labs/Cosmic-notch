// SPDX-License-Identifier: GPL-3.0-only

//! Month grid for the calendar tab.

use chrono::{Datelike, Locale, Months, NaiveDate, Weekday};

/// Weeks of a month, each with seven days starting at `first_day`; days of
/// neighbouring months are `None`.
pub fn month_grid(year: i32, month: u32, first_day: Weekday) -> Vec<[Option<NaiveDate>; 7]> {
    let Some(first) = NaiveDate::from_ymd_opt(year, month, 1) else {
        return Vec::new();
    };
    let offset = first.weekday().days_since(first_day) as usize;
    let mut weeks = Vec::new();
    let mut week = [None; 7];
    let mut slot = offset;
    for date in first.iter_days().take_while(|d| d.month() == month) {
        week[slot] = Some(date);
        slot += 1;
        if slot == 7 {
            weeks.push(week);
            week = [None; 7];
            slot = 0;
        }
    }
    if slot > 0 {
        weeks.push(week);
    }
    weeks
}

/// The first day of the month `delta` months away from `date`'s month.
pub fn shift_month(date: NaiveDate, delta: i32) -> NaiveDate {
    let first = date.with_day(1).unwrap_or(date);
    if delta >= 0 {
        first.checked_add_months(Months::new(delta as u32))
    } else {
        first.checked_sub_months(Months::new(delta.unsigned_abs()))
    }
    .unwrap_or(first)
}

/// Weekday as used by the COSMIC time applet: 0 = Monday … 6 = Sunday.
pub fn weekday_from_config(day: u8) -> Weekday {
    Weekday::try_from(day).unwrap_or(Weekday::Mon)
}

/// Short weekday names in display order, e.g. `po út st čt pá so ne`.
pub fn weekday_names(first_day: Weekday, locale: Locale) -> [String; 7] {
    // 2024-01-01 was a Monday.
    let monday = NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid date");
    std::array::from_fn(|i| {
        let day = monday + chrono::Days::new(u64::from(first_day.num_days_from_monday()) + i as u64);
        let name = super::clock::format(
            &day.and_hms_opt(0, 0, 0).expect("valid time").and_utc(),
            "%a",
            locale,
        );
        name.chars().take(2).collect()
    })
}

/// Nominative month names for languages where chrono's `%B` is the genitive
/// ("října" instead of "říjen").
fn nominative_months(locale: Locale) -> Option<[&'static str; 12]> {
    let lang = format!("{locale:?}");
    Some(match lang.get(..2)? {
        "cs" => [
            "leden", "únor", "březen", "duben", "květen", "červen", "červenec", "srpen", "září",
            "říjen", "listopad", "prosinec",
        ],
        "sk" => [
            "január", "február", "marec", "apríl", "máj", "jún", "júl", "august", "september",
            "október", "november", "december",
        ],
        "pl" => [
            "styczeń", "luty", "marzec", "kwiecień", "maj", "czerwiec", "lipiec", "sierpień",
            "wrzesień", "październik", "listopad", "grudzień",
        ],
        _ => return None,
    })
}

/// Month and year heading, e.g. `Říjen 2026`.
pub fn month_title(date: NaiveDate, locale: Locale) -> String {
    let title = match nominative_months(locale) {
        Some(months) => format!("{} {}", months[date.month0() as usize], date.year()),
        None => date.format_localized("%B %Y", locale).to_string(),
    };
    let mut chars = title.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> Option<NaiveDate> {
        NaiveDate::from_ymd_opt(y, m, d)
    }

    #[test]
    fn october_2026_starting_monday() {
        let weeks = month_grid(2026, 10, Weekday::Mon);
        // 1. 10. 2026 is a Thursday.
        assert_eq!(weeks[0][..3], [None, None, None]);
        assert_eq!(weeks[0][3], day(2026, 10, 1));
        assert_eq!(weeks.len(), 5);
        assert_eq!(weeks[4][5], day(2026, 10, 31));
        assert_eq!(weeks[4][6], None);
    }

    #[test]
    fn sunday_start_shifts_by_one() {
        let weeks = month_grid(2026, 10, Weekday::Sun);
        assert_eq!(weeks[0][4], day(2026, 10, 1));
    }

    #[test]
    fn leap_february() {
        let days: usize = month_grid(2028, 2, Weekday::Mon)
            .iter()
            .map(|w| w.iter().flatten().count())
            .sum();
        assert_eq!(days, 29);
        let days: usize = month_grid(2026, 2, Weekday::Mon)
            .iter()
            .map(|w| w.iter().flatten().count())
            .sum();
        assert_eq!(days, 28);
    }

    #[test]
    fn shifting_months_across_years() {
        let d = NaiveDate::from_ymd_opt(2026, 1, 31).unwrap();
        assert_eq!(shift_month(d, -1), NaiveDate::from_ymd_opt(2025, 12, 1).unwrap());
        assert_eq!(shift_month(d, 13), NaiveDate::from_ymd_opt(2027, 2, 1).unwrap());
    }

    #[test]
    fn czech_names() {
        assert_eq!(
            weekday_names(Weekday::Mon, Locale::cs_CZ).join(" "),
            "po út st čt pá so ne"
        );
        let october = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        assert_eq!(month_title(october, Locale::cs_CZ), "Říjen 2026");
    }
}
