// SPDX-License-Identifier: GPL-3.0-only

//! Events of the primary Google Calendar, through GNOME Online Accounts (see
//! [`super::google`]): read for the days the calendar shows, and created from the island.

use chrono::{DateTime, Duration, Local, NaiveDate, NaiveTime, TimeZone};
use serde::Deserialize;

use super::google::{CLIENT, Service, check, token};
pub use super::google::{Account, Error};

const EVENTS: &str = "https://www.googleapis.com/calendar/v3/calendars/primary/events";

/// When an event starts or ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum When {
    /// All day: just the date.
    Day(NaiveDate),
    At(DateTime<Local>),
}

impl When {
    pub fn date(self) -> NaiveDate {
        match self {
            When::Day(date) => date,
            When::At(at) => at.date_naive(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub id: String,
    pub title: String,
    pub start: When,
    /// Where it ends; for all-day events, the day after the last one (as Google has it).
    pub end: When,
}

impl Event {
    pub fn all_day(&self) -> bool {
        matches!(self.start, When::Day(_))
    }

    /// The days the event touches.
    pub fn days(&self) -> Vec<NaiveDate> {
        let first = self.start.date();
        let mut last = self.end.date();
        let ends_at_midnight = match self.end {
            When::Day(_) => true,
            When::At(at) => at.time() == NaiveTime::MIN,
        };
        // An end at midnight is the start of the next day, which the event doesn't touch.
        if ends_at_midnight && last > first {
            last -= Duration::days(1);
        }
        let mut days = Vec::new();
        let mut day = first;
        while day <= last && days.len() < 62 {
            days.push(day);
            day += Duration::days(1);
        }
        days
    }
}

#[derive(Debug, Deserialize)]
struct RemoteWhen {
    date: Option<NaiveDate>,
    #[serde(rename = "dateTime")]
    date_time: Option<DateTime<chrono::FixedOffset>>,
}

impl RemoteWhen {
    fn parse(&self) -> Option<When> {
        match (self.date, self.date_time) {
            (_, Some(at)) => Some(When::At(at.with_timezone(&Local))),
            (Some(day), None) => Some(When::Day(day)),
            _ => None,
        }
    }
}

#[derive(Debug, Deserialize)]
struct RemoteEvent {
    id: String,
    #[serde(default)]
    summary: String,
    start: Option<RemoteWhen>,
    end: Option<RemoteWhen>,
    #[serde(default)]
    status: String,
}

impl RemoteEvent {
    fn into_event(self) -> Option<Event> {
        if self.status == "cancelled" {
            return None;
        }
        let start = self.start?.parse()?;
        let end = self.end.and_then(|e| e.parse()).unwrap_or(start);
        Some(Event {
            id: self.id,
            title: self.summary,
            start,
            end,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Page {
    #[serde(default)]
    items: Vec<RemoteEvent>,
    next_page_token: Option<String>,
}

/// The first Google account in Online Accounts whose calendar is switched on.
pub async fn account() -> Result<Account, Error> {
    super::google::account(Service::Calendar).await
}

fn midnight(day: NaiveDate) -> DateTime<Local> {
    // Midnight can fall in a daylight saving gap; the next hour is close enough.
    Local
        .from_local_datetime(&day.and_time(NaiveTime::MIN))
        .earliest()
        .unwrap_or_else(|| Local.from_utc_datetime(&day.and_time(NaiveTime::MIN)))
}

/// Events from `from` up to, not including, `to`, in the order they start.
pub async fn events(from: NaiveDate, to: NaiveDate) -> Result<(Account, Vec<Event>), Error> {
    let account = account().await?;
    let token = token(&account).await?;
    let mut events = Vec::new();
    let mut page_token: Option<String> = None;
    loop {
        let mut query = vec![
            ("singleEvents", "true".to_string()),
            ("orderBy", "startTime".to_string()),
            ("timeMin", midnight(from).to_rfc3339()),
            ("timeMax", midnight(to).to_rfc3339()),
            ("maxResults", "250".to_string()),
        ];
        if let Some(page) = page_token.take() {
            query.push(("pageToken", page));
        }
        let response = CLIENT
            .get(EVENTS)
            .query(&query)
            .bearer_auth(&token)
            .send()
            .await?;
        let page: Page = check(response).await?.json().await?;
        events.extend(page.items.into_iter().filter_map(RemoteEvent::into_event));
        match page.next_page_token {
            Some(next) => page_token = Some(next),
            None => break,
        }
    }
    Ok((account, events))
}

/// An event to create.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewEvent {
    pub title: String,
    pub date: NaiveDate,
    /// Start and end on that day; `None` for an all-day event.
    pub times: Option<(NaiveTime, NaiveTime)>,
}

impl NewEvent {
    fn body(&self) -> serde_json::Value {
        match self.times {
            None => serde_json::json!({
                "summary": self.title,
                "start": { "date": self.date.to_string() },
                "end": { "date": (self.date + Duration::days(1)).to_string() },
            }),
            Some((start, end)) => {
                let at = |time: NaiveTime| {
                    Local
                        .from_local_datetime(&self.date.and_time(time))
                        .earliest()
                        .unwrap_or_else(|| midnight(self.date))
                        .to_rfc3339()
                };
                // An end before the start means the event runs past midnight.
                let end_day = if end <= start { self.date + Duration::days(1) } else { self.date };
                let end_at = Local
                    .from_local_datetime(&end_day.and_time(end))
                    .earliest()
                    .unwrap_or_else(|| midnight(end_day))
                    .to_rfc3339();
                serde_json::json!({
                    "summary": self.title,
                    "start": { "dateTime": at(start) },
                    "end": { "dateTime": end_at },
                })
            }
        }
    }
}

/// Creates the event in the primary calendar.
pub async fn create(new: NewEvent) -> Result<Event, Error> {
    let account = account().await?;
    let token = token(&account).await?;
    let response = CLIENT
        .post(EVENTS)
        .json(&new.body())
        .bearer_auth(token)
        .send()
        .await?;
    let remote: RemoteEvent = check(response).await?.json().await?;
    remote
        .into_event()
        .ok_or_else(|| Error::Other("Google returned an event without a start".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn parses_timed_and_all_day_events() {
        let page = r#"{"items":[
            {"id":"a","summary":"Dentist","status":"confirmed",
             "start":{"dateTime":"2026-10-26T14:30:00+01:00"},"end":{"dateTime":"2026-10-26T15:00:00+01:00"}},
            {"id":"b","summary":"Holiday","start":{"date":"2026-10-28"},"end":{"date":"2026-10-30"}},
            {"id":"c","summary":"Gone","status":"cancelled","start":{"date":"2026-10-28"},"end":{"date":"2026-10-29"}}]}"#;
        let page: Page = serde_json::from_str(page).unwrap();
        let events: Vec<_> = page.items.into_iter().filter_map(RemoteEvent::into_event).collect();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].title, "Dentist");
        assert!(!events[0].all_day());
        assert!(events[1].all_day());
        // Google's end date of an all-day event is exclusive.
        assert_eq!(events[1].days(), [day(2026, 10, 28), day(2026, 10, 29)]);
    }

    #[test]
    fn all_day_event_body_ends_the_next_day() {
        let new = NewEvent {
            title: "Trip".into(),
            date: day(2026, 12, 31),
            times: None,
        };
        let body = new.body();
        assert_eq!(body["start"]["date"], "2026-12-31");
        assert_eq!(body["end"]["date"], "2027-01-01");
    }

    #[test]
    fn timed_event_past_midnight_ends_the_next_day() {
        let new = NewEvent {
            title: "Party".into(),
            date: day(2026, 12, 31),
            times: Some((
                NaiveTime::from_hms_opt(22, 0, 0).unwrap(),
                NaiveTime::from_hms_opt(1, 0, 0).unwrap(),
            )),
        };
        let body = new.body();
        let end = body["end"]["dateTime"].as_str().unwrap();
        assert!(end.starts_with("2027-01-01T01:00"), "{end}");
    }

    /// Needs a Google account in GNOME Online Accounts; prints only how many events there are.
    #[tokio::test]
    #[ignore]
    async fn lists_events() {
        let today = Local::now().date_naive();
        let (account, events) = events(today, today + Duration::days(30)).await.unwrap();
        println!("{}: {} events in the next 30 days", account.name, events.len());
    }
}
