// SPDX-License-Identifier: GPL-3.0-only

//! Keeping the tasks in step with Google Tasks. Changes made here go to Google
//! right away; Google's list is pulled every minute and when the tab opens.

use std::collections::{HashMap, HashSet};

use cosmic::app::Task;

use super::{Island, Message};
use crate::modules::google_tasks::{self, Change, Error};

/// Where syncing with Google stands.
#[derive(Debug, Default)]
pub struct GoogleSync {
    /// The account last synced with.
    pub account: Option<String>,
    pub error: Option<Error>,
    /// Local tasks being uploaded.
    uploading: HashSet<u64>,
    /// Google tasks being deleted, so a pull meanwhile doesn't bring them back.
    deleting: HashSet<String>,
    /// New titles being sent, so a pull meanwhile doesn't show the old ones.
    renaming: HashMap<String, String>,
    pulling: bool,
}

/// Result of sending a change to Google.
#[derive(Debug, Clone)]
pub struct Applied {
    change: Change,
    /// The local task a created one belongs to.
    local: Option<u64>,
    result: Result<Option<String>, Error>,
}

impl Island {
    fn syncing(&self) -> bool {
        self.config.google_tasks
    }

    /// Uploads tasks Google doesn't have yet and pulls Google's list.
    pub(super) fn sync_tasks(&mut self) -> Task<Message> {
        if !self.syncing() {
            return Task::none();
        }
        let pending: Vec<_> = self
            .todos
            .items
            .iter()
            .filter(|t| t.google_id.is_none() && !t.done && !self.google.uploading.contains(&t.id))
            .map(|t| (t.id, t.text.clone()))
            .collect();
        // Oldest first, so they end up in Google in the same order.
        let mut tasks: Vec<_> = pending
            .into_iter()
            .rev()
            .map(|(id, title)| self.send(Change::Create { title }, Some(id)))
            .collect();
        if !self.google.pulling {
            self.google.pulling = true;
            tasks.push(cosmic::task::future(async {
                Message::GooglePulled(
                    google_tasks::list()
                        .await
                        .map(|(account, tasks)| {
                            let tasks = tasks.into_iter().map(|t| (t.id, t.title)).collect();
                            (account.name, tasks)
                        }),
                )
            }));
        }
        Task::batch(tasks)
    }

    pub(super) fn pulled(&mut self, result: Result<(String, Vec<(String, String)>), Error>) {
        self.google.pulling = false;
        match result {
            Ok((account, remote)) => {
                self.google.account = Some(account);
                self.google.error = None;
                let remote = remote
                    .into_iter()
                    .filter(|(id, _)| !self.google.deleting.contains(id))
                    .map(|(id, title)| match self.google.renaming.get(&id) {
                        Some(title) => (id, title.clone()),
                        None => (id, title),
                    })
                    .collect();
                // Text being edited stays as the user left it.
                self.todos.merge_remote(remote);
                self.save_todos();
            }
            Err(err) => self.google.error = Some(err),
        }
    }

    /// Sends a change for a local task to Google, if it is there already.
    pub(super) fn push_change(&mut self, id: u64, change: impl FnOnce(String) -> Change) -> Task<Message> {
        if !self.syncing() {
            return Task::none();
        }
        match self.todos.google_id(id) {
            Some(google_id) => self.send(change(google_id), None),
            // Not uploaded yet: the upload takes the task as it is then.
            None => Task::none(),
        }
    }

    pub(super) fn send(&mut self, change: Change, local: Option<u64>) -> Task<Message> {
        match &change {
            Change::Create { .. } => {
                if let Some(id) = local {
                    self.google.uploading.insert(id);
                }
            }
            Change::Delete { id } => {
                self.google.deleting.insert(id.clone());
            }
            Change::Rename { id, title } => {
                self.google.renaming.insert(id.clone(), title.clone());
            }
            Change::Complete { .. } => {}
        }
        cosmic::task::future(async move {
            let result = google_tasks::apply(change.clone()).await;
            Message::GoogleApplied(Box::new(Applied {
                change,
                local,
                result,
            }))
        })
    }

    pub(super) fn applied(&mut self, applied: Applied) -> Task<Message> {
        let Applied {
            change,
            local,
            result,
        } = applied;
        match &change {
            Change::Delete { id } => {
                self.google.deleting.remove(id);
            }
            Change::Rename { id, title } if self.google.renaming.get(id) == Some(title) => {
                self.google.renaming.remove(id);
            }
            _ => {}
        }
        let created = match result {
            Ok(created) => {
                self.google.error = None;
                created
            }
            Err(err) => {
                tracing::warn!("Google Tasks: {change:?} failed: {err:?}");
                self.google.error = Some(err);
                if let Some(id) = local {
                    self.google.uploading.remove(&id);
                }
                return Task::none();
            }
        };
        let (Some(id), Some(google_id)) = (local, created) else {
            return Task::none();
        };
        self.google.uploading.remove(&id);
        // The task may have changed while it was uploading.
        let Some(todo) = self.todos.items.iter().find(|t| t.id == id).cloned() else {
            return self.send(Change::Delete { id: google_id }, None);
        };
        self.todos.set_google_id(id, google_id.clone());
        self.save_todos();
        let mut tasks = Vec::new();
        if matches!(&change, Change::Create { title } if *title != todo.text) {
            tasks.push(self.send(Change::Rename { id: google_id.clone(), title: todo.text }, None));
        }
        if todo.done {
            tasks.push(self.send(Change::Complete { id: google_id, done: true }, None));
        }
        Task::batch(tasks)
    }
}
