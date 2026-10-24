// SPDX-License-Identifier: GPL-3.0-only

//! Google Tasks through GNOME Online Accounts (see [`super::google`]).
//! The island syncs the default list ("My Tasks").

use serde::Deserialize;

use super::google::{CLIENT, Service, check, token};
pub use super::google::{Account, Error};

const TASKS: &str = "https://tasks.googleapis.com/tasks/v1/lists/@default/tasks";

/// A task as Google keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RemoteTask {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub position: String,
}

/// The first Google account in Online Accounts whose tasks are switched on.
pub async fn account() -> Result<Account, Error> {
    super::google::account(Service::Tasks).await
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
