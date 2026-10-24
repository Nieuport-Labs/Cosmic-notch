// SPDX-License-Identifier: GPL-3.0-only

//! Events of the Google Calendar, shown in the calendar tab, and the form that adds one.
//! Fetched for the month on screen when the tab opens, every minute while it is
//! open and now and then otherwise (the home page shows the next event).

use std::collections::HashMap;

use chrono::{Datelike, Duration, NaiveDate, NaiveTime};
use cosmic::app::Task;

use super::{Island, Message};
use crate::modules::calendar;
use crate::modules::google_calendar::{self, Error, Event, NewEvent};

pub fn event_title_input_id() -> cosmic::widget::Id {
    cosmic::widget::Id::new("event-title")
}

/// The form for a new event.
#[derive(Debug, Clone)]
pub struct EventForm {
    pub title: String,
    pub all_day: bool,
    /// `H:MM` or `HH:MM`, as typed.
    pub start: String,
    pub end: String,
    pub error: Option<String>,
    pub creating: bool,
}

impl EventForm {
    pub fn new(now: chrono::DateTime<chrono::Local>) -> Self {
        use chrono::Timelike;
        // The coming full hour, for an hour.
        let start = now.time().with_minute(0).unwrap_or(now.time()) + Duration::hours(1);
        let end = start + Duration::hours(1);
        Self {
            title: String::new(),
            all_day: false,
            start: start.format("%H:%M").to_string(),
            end: end.format("%H:%M").to_string(),
            error: None,
            creating: false,
        }
    }

    /// The event the form describes, on `date`; the error says what to fix.
    pub fn event(&self, date: NaiveDate) -> Result<NewEvent, String> {
        let title = self.title.trim();
        if title.is_empty() {
            return Err(crate::fl!("event-needs-title"));
        }
        let times = if self.all_day {
            None
        } else {
            let parse = |text: &str| {
                parse_time(text).ok_or_else(|| crate::fl!("event-bad-time", time = text.to_owned()))
            };
            Some((parse(&self.start)?, parse(&self.end)?))
        };
        Ok(NewEvent {
            title: title.to_owned(),
            date,
            times,
        })
    }
}

/// `9`, `9:30` or `09:30`.
pub fn parse_time(text: &str) -> Option<NaiveTime> {
    let text = text.trim();
    let (hours, minutes) = text.split_once([':', '.']).unwrap_or((text, "0"));
    NaiveTime::from_hms_opt(hours.parse().ok()?, minutes.parse().ok()?, 0)
}

#[derive(Debug)]
pub struct CalendarSync {
    /// Events by the days they touch.
    pub events: HashMap<NaiveDate, Vec<Event>>,
    /// The days they were fetched for: first, and the day after the last.
    pub loaded: Option<(NaiveDate, NaiveDate)>,
    pub account: Option<String>,
    pub error: Option<Error>,
    fetching: bool,
    /// The day whose events the tab lists.
    pub selected: NaiveDate,
    pub form: Option<EventForm>,
}

impl CalendarSync {
    pub fn new(today: NaiveDate) -> Self {
        Self {
            events: HashMap::new(),
            loaded: None,
            account: None,
            error: None,
            fetching: false,
            selected: today,
            form: None,
        }
    }

    /// Events of a day, all-day ones first, then by start.
    pub fn on(&self, day: NaiveDate) -> &[Event] {
        self.events.get(&day).map_or(&[], Vec::as_slice)
    }

    fn insert(&mut self, event: Event) {
        for day in event.days() {
            let list = self.events.entry(day).or_default();
            if !list.iter().any(|e| e.id == event.id) {
                list.push(event.clone());
                sort_day(list);
            }
        }
    }
}

fn sort_day(list: &mut [Event]) {
    list.sort_by_key(|e| (!e.all_day(), e.start.date(), match e.start {
        google_calendar::When::At(at) => at.time(),
        google_calendar::When::Day(_) => NaiveTime::MIN,
    }));
}

/// The days to fetch for the month shown: the month, and the next two weeks
/// from `today` when they are near, for the home page.
pub fn range(month: NaiveDate, today: NaiveDate) -> (NaiveDate, NaiveDate) {
    let from = month;
    let to = calendar::shift_month(month, 1);
    let soon = today + Duration::days(14);
    let near = today >= calendar::shift_month(month, -1) && today < calendar::shift_month(month, 2);
    if near {
        (from.min(today), to.max(soon))
    } else {
        (from, to)
    }
}

impl Island {
    /// Fetches the events around the month on screen, when Google Calendar is on.
    pub(super) fn sync_calendar(&mut self) -> Task<Message> {
        if !self.config.google_calendar || self.calendar.fetching {
            return Task::none();
        }
        self.calendar.fetching = true;
        let (from, to) = range(self.calendar_month, self.now.date_naive());
        cosmic::task::future(async move {
            Message::CalendarPulled(from, to, google_calendar::events(from, to).await.map(|(a, e)| (a.name, e)))
        })
    }

    pub(super) fn calendar_pulled(
        &mut self,
        from: NaiveDate,
        to: NaiveDate,
        result: Result<(String, Vec<Event>), Error>,
    ) -> Task<Message> {
        self.calendar.fetching = false;
        match result {
            Ok((account, events)) => {
                self.calendar.events.clear();
                for event in events {
                    self.calendar.insert(event);
                }
                self.calendar.loaded = Some((from, to));
                self.calendar.account = Some(account);
                self.calendar.error = None;
            }
            Err(err) => self.calendar.error = Some(err),
        }
        // The month may have changed while fetching.
        let (want_from, want_to) = range(self.calendar_month, self.now.date_naive());
        if self.config.google_calendar && (want_from, want_to) != (from, to) {
            return self.sync_calendar();
        }
        Task::none()
    }

    /// Picks the day the tab lists events of.
    pub(super) fn select_day(&mut self, day: NaiveDate) {
        self.calendar.selected = day;
        // The form belongs to the day it was opened on.
        self.calendar.form = None;
    }

    pub(super) fn submit_event(&mut self) -> Task<Message> {
        let day = self.calendar.selected;
        let Some(form) = &mut self.calendar.form else {
            return Task::none();
        };
        if form.creating {
            return Task::none();
        }
        match form.event(day) {
            Ok(new) => {
                form.creating = true;
                form.error = None;
                cosmic::task::future(async move { Message::EventCreated(google_calendar::create(new).await) })
            }
            Err(error) => {
                form.error = Some(error);
                Task::none()
            }
        }
    }

    pub(super) fn event_created(&mut self, result: Result<Event, Error>) -> Task<Message> {
        match result {
            Ok(event) => {
                self.calendar.form = None;
                self.calendar.error = None;
                self.calendar.insert(event);
                // Pick up what Google made of it (recurrence, ids).
                self.sync_calendar()
            }
            Err(err) => {
                if let Some(form) = &mut self.calendar.form {
                    form.creating = false;
                    form.error = Some(match err {
                        Error::NoPermission => crate::fl!("google-no-permission"),
                        Error::NoAccount => crate::fl!("google-no-account"),
                        Error::Other(error) => crate::fl!("google-error", error = error),
                    });
                }
                Task::none()
            }
        }
    }

    /// The next event from now on: one that has not ended, today or in the next days.
    pub fn next_event(&self) -> Option<&Event> {
        use crate::modules::google_calendar::When;
        let now = self.now;
        let today = now.date_naive();
        let (from, to) = self.calendar.loaded?;
        if today < from || today >= to {
            return None;
        }
        let mut day = today;
        while day < to && day <= today + Duration::days(14) {
            let found = self.calendar.on(day).iter().find(|e| match e.end {
                When::At(end) => end > now,
                When::Day(_) => true,
            });
            if found.is_some() {
                return found;
            }
            day += Duration::days(1);
        }
        None
    }

    /// One line on how Google Calendar goes, for the settings.
    pub(super) fn calendar_status(&self) -> String {
        match (&self.calendar.error, &self.calendar.account) {
            (Some(Error::NoAccount), _) => crate::fl!("google-no-account"),
            (Some(Error::NoPermission), _) => crate::fl!("google-no-permission"),
            (Some(Error::Other(err)), _) => crate::fl!("google-error", error = err.clone()),
            (None, Some(account)) if self.config.google_calendar => {
                crate::fl!("google-account", account = account.clone())
            }
            _ => crate::fl!("google-calendar-hint"),
        }
    }

    /// Whether `month` is the month with `day` in it.
    pub fn in_month(month: NaiveDate, day: NaiveDate) -> bool {
        month.year() == day.year() && month.month() == day.month()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn parses_times_as_typed() {
        assert_eq!(parse_time("9"), NaiveTime::from_hms_opt(9, 0, 0));
        assert_eq!(parse_time("9:30"), NaiveTime::from_hms_opt(9, 30, 0));
        assert_eq!(parse_time(" 18.05 "), NaiveTime::from_hms_opt(18, 5, 0));
        assert_eq!(parse_time("25:00"), None);
        assert_eq!(parse_time("soon"), None);
    }

    #[test]
    fn fetches_the_next_two_weeks_when_near() {
        let today = day(2026, 10, 30);
        // October on screen: the month, and the days after it that the home page wants.
        assert_eq!(range(day(2026, 10, 1), today), (day(2026, 10, 1), day(2026, 11, 13)));
        // A month far away: only that month.
        assert_eq!(range(day(2027, 5, 1), today), (day(2027, 5, 1), day(2027, 6, 1)));
    }

    #[test]
    fn a_form_needs_a_title_and_times() {
        let mut form = EventForm::new(chrono::Local::now());
        assert!(form.event(day(2026, 10, 30)).is_err());
        form.title = " Dentist ".into();
        form.start = "14:30".into();
        form.end = "15".into();
        let new = form.event(day(2026, 10, 30)).unwrap();
        assert_eq!(new.title, "Dentist");
        assert_eq!(new.times, Some((NaiveTime::from_hms_opt(14, 30, 0).unwrap(), NaiveTime::from_hms_opt(15, 0, 0).unwrap())));
        form.end = "later".into();
        assert!(form.event(day(2026, 10, 30)).is_err());
        form.all_day = true;
        assert_eq!(form.event(day(2026, 10, 30)).unwrap().times, None);
    }

    #[test]
    fn multi_day_events_show_on_each_day() {
        use crate::modules::google_calendar::When;
        let mut sync = CalendarSync::new(day(2026, 10, 28));
        sync.insert(Event {
            id: "h".into(),
            title: "Holiday".into(),
            start: When::Day(day(2026, 10, 28)),
            end: When::Day(day(2026, 10, 30)),
        });
        assert_eq!(sync.on(day(2026, 10, 28)).len(), 1);
        assert_eq!(sync.on(day(2026, 10, 29)).len(), 1);
        assert!(sync.on(day(2026, 10, 30)).is_empty());
    }
}
