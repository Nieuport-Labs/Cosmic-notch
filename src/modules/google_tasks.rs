// SPDX-License-Identifier: GPL-3.0-only

//! Google Tasks through GNOME Online Accounts: GOA signs in to Google and
//! hands out access tokens, so the island needs no Google client of its own.
//! The island syncs the default list ("My Tasks").

use std::collections::HashMap;
use std::sync::LazyLock;

use serde::Deserialize;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

const GOA: &str = "org.gnome.OnlineAccounts";
const ACCOUNT: &str = "org.gnome.OnlineAccounts.Account";
const OAUTH2: &str = "org.gnome.OnlineAccounts.OAuth2Based";
const TASKS: &str = "https://tasks.googleapis.com/tasks/v1/lists/@default/tasks";

/// A Google account set up in GNOME Online Accounts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    path: OwnedObjectPath,
    /// The address the user knows the account by.
    pub name: String,
}

/// A task as Google keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RemoteTask {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub position: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// No Google account in Online Accounts, or its tasks are switched off there.
    NoAccount,
    /// The account was added before GOA asked Google for tasks.
    NoPermission,
    Other(String),
}

impl<E: std::fmt::Display> From<E> for Error {
    fn from(err: E) -> Self {
        Error::Other(err.to_string())
    }
}

/// One client for all requests, so its connection to Google is reused.
static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(reqwest::Client::new);

type Objects = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;

/// The first Google account in Online Accounts whose tasks are switched on.
pub async fn account() -> Result<Account, Error> {
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
                && !flag("TodoDisabled")
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

async fn token(account: &Account) -> Result<String, Error> {
    let connection = super::media::connection().await?;
    let reply = connection
        .call_method(Some(GOA), &account.path, Some(OAUTH2), "GetAccessToken", &())
        .await?;
    let (token, _expires_in): (String, i32) = reply.body().deserialize()?;
    Ok(token)
}

/// What to change on Google.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Create { title: String },
    Rename { id: String, title: String },
    Complete { id: String, done: bool },
    Delete { id: String },
}

/// Sends one change; a created task comes back with its id.
pub async fn apply(change: Change) -> Result<Option<String>, Error> {
    let account = account().await?;
    let token = token(&account).await?;
    let client = &*CLIENT;
    let request = match &change {
        Change::Create { title } => client
            .post(TASKS)
            .json(&serde_json::json!({ "title": title })),
        Change::Rename { id, title } => client
            .patch(format!("{TASKS}/{id}"))
            .json(&serde_json::json!({ "title": title })),
        Change::Complete { id, done } => client.patch(format!("{TASKS}/{id}")).json(&if *done {
            serde_json::json!({ "status": "completed" })
        } else {
            serde_json::json!({ "status": "needsAction", "completed": null })
        }),
        Change::Delete { id } => client.delete(format!("{TASKS}/{id}")),
    };
    let response = check(request.bearer_auth(token).send().await?).await;
    // Gone already is as good as deleted.
    if matches!(change, Change::Delete { .. })
        && matches!(&response, Err(Error::Other(e)) if e.starts_with("404"))
    {
        return Ok(None);
    }
    let response = response?;
    if let Change::Create { .. } = change {
        let task: RemoteTask = response.json().await?;
        return Ok(Some(task.id));
    }
    Ok(None)
}

/// Open tasks of the default list, in Google's order.
pub async fn list() -> Result<(Account, Vec<RemoteTask>), Error> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Page {
        #[serde(default)]
        items: Vec<RemoteTask>,
        next_page_token: Option<String>,
    }

    let account = account().await?;
    let token = token(&account).await?;
    let client = &*CLIENT;
    let mut tasks = Vec::new();
    let mut page_token: Option<String> = None;
    loop {
        let mut query = vec![
            ("showCompleted", "false".to_string()),
            ("showHidden", "false".to_string()),
            ("maxResults", "100".to_string()),
        ];
        if let Some(page) = page_token.take() {
            query.push(("pageToken", page));
        }
        let response = client
            .get(TASKS)
            .query(&query)
            .bearer_auth(&token)
            .send()
            .await?;
        let page: Page = check(response).await?.json().await?;
        tasks.extend(page.items);
        match page.next_page_token {
            Some(next) => page_token = Some(next),
            None => break,
        }
    }
    // Subtasks have their own positions; ordering by position keeps each under its parent closely enough.
    tasks.sort_by(|a, b| a.position.cmp(&b.position));
    Ok((account, tasks))
}

async fn check(response: reqwest::Response) -> Result<reqwest::Response, Error> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_tasks_page() {
        let page = r#"{"kind":"tasks#tasks","items":[
            {"id":"b","title":"Druhý","position":"00000000000000000001","status":"needsAction"},
            {"id":"a","title":"","position":"00000000000000000000"}]}"#;
        #[derive(Deserialize)]
        struct Page {
            items: Vec<RemoteTask>,
        }
        let page: Page = serde_json::from_str(page).unwrap();
        assert_eq!(page.items[0].title, "Druhý");
        assert_eq!(page.items[1].title, "");
    }

    /// Needs a Google account in GNOME Online Accounts.
    #[tokio::test]
    #[ignore]
    async fn lists_tasks() {
        let (account, tasks) = list().await.unwrap();
        println!("{}: {tasks:#?}", account.name);
    }
}
