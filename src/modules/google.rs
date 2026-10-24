// SPDX-License-Identifier: GPL-3.0-only

//! Google through GNOME Online Accounts: GOA signs in to Google and hands out
//! access tokens, so the island needs no Google client of its own. Used by
//! [`super::google_tasks`] and [`super::google_calendar`].

use std::collections::HashMap;
use std::sync::LazyLock;

use zbus::zvariant::{OwnedObjectPath, OwnedValue};

const GOA: &str = "org.gnome.OnlineAccounts";
const ACCOUNT: &str = "org.gnome.OnlineAccounts.Account";
const OAUTH2: &str = "org.gnome.OnlineAccounts.OAuth2Based";

/// A Google account set up in GNOME Online Accounts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    path: OwnedObjectPath,
    /// The address the user knows the account by.
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// No Google account in Online Accounts, or the service is switched off there.
    NoAccount,
    /// The account was added before GOA asked Google for this service.
    NoPermission,
    Other(String),
}

impl<E: std::fmt::Display> From<E> for Error {
    fn from(err: E) -> Self {
        Error::Other(err.to_string())
    }
}

/// What the account is used for; each can be switched off per account in Online Accounts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Service {
    Tasks,
    Calendar,
}

impl Service {
    /// The account property that says the service is off.
    fn disabled_flag(self) -> &'static str {
        match self {
            Service::Tasks => "TodoDisabled",
            Service::Calendar => "CalendarDisabled",
        }
    }
}

/// One client for all requests, so its connection to Google is reused.
pub static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(reqwest::Client::new);

type Objects = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;

/// The first Google account in Online Accounts that has `service` switched on.
pub async fn account(service: Service) -> Result<Account, Error> {
    let connection = super::media::connection().await?;
    let reply = connection
        .call_method(
            Some(GOA),
            "/org/gnome/OnlineAccounts",
            Some("org.freedesktop.DBus.ObjectManager"),
            "GetManagedObjects",
            &(),
        )
        .await
        // Without the daemon there are no online accounts.
        .map_err(|_| Error::NoAccount)?;
    let objects: Objects = reply.body().deserialize()?;
    let mut accounts: Vec<_> = objects
        .into_iter()
        .filter_map(|(path, interfaces)| {
            let account = interfaces.get(ACCOUNT)?;
            let text = |key: &str| {
                account
                    .get(key)
                    .and_then(|v| String::try_from(v.try_clone().ok()?).ok())
                    .unwrap_or_default()
            };
            let flag = |key: &str| {
                account
                    .get(key)
                    .and_then(|v| bool::try_from(v).ok())
                    .unwrap_or(false)
            };
            (text("ProviderType") == "google"
                && !flag(service.disabled_flag())
                && interfaces.contains_key(OAUTH2))
            .then(|| Account {
                path,
                name: text("PresentationIdentity"),
            })
        })
        .collect();
    accounts.sort_by(|a, b| a.path.as_str().cmp(b.path.as_str()));
    accounts.into_iter().next().ok_or(Error::NoAccount)
}

/// A fresh access token for the account.
pub async fn token(account: &Account) -> Result<String, Error> {
    let connection = super::media::connection().await?;
    let reply = connection
        .call_method(Some(GOA), &account.path, Some(OAUTH2), "GetAccessToken", &())
        .await?;
    let (token, _expires_in): (String, i32) = reply.body().deserialize()?;
    Ok(token)
}

/// Passes a successful response on; turns the others into errors.
pub async fn check(response: reqwest::Response) -> Result<reqwest::Response, Error> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response.text().await.unwrap_or_default();
    if status == reqwest::StatusCode::FORBIDDEN && body.contains("insufficient") {
        return Err(Error::NoPermission);
    }
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(Error::NoPermission);
    }
    Err(Error::Other(format!("{} {}", status.as_u16(), status.canonical_reason().unwrap_or(""))))
}
