// SPDX-License-Identifier: GPL-3.0-only

//! A to-do list, newest on top. Like Google Tasks, a ticked task is shown
//! ticked for a moment and then removed.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Todo {
    pub id: u64,
    pub text: String,
    pub done: bool,
    /// The task's id in Google Tasks, once it is there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub google_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Todos {
    pub items: Vec<Todo>,
    next_id: u64,
}

impl Todos {
    pub fn from_stored(items: Vec<Todo>) -> Self {
        let next_id = items.iter().map(|t| t.id + 1).max().unwrap_or(0);
        let mut todos = Self { items, next_id };
        // Ticked tasks go away; one ticked just before quitting too.
        todos.clear_done();
        todos
    }

    /// Adds a task on top; blank text adds nothing.
    pub fn add(&mut self, text: &str) -> Option<u64> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.items.insert(
            0,
            Todo {
                id,
                text: text.to_string(),
                done: false,
                google_id: None,
            },
        );
        Some(id)
    }

    /// Ticks or unticks a task in place.
    pub fn set_done(&mut self, id: u64, done: bool) {
        if let Some(todo) = self.items.iter_mut().find(|t| t.id == id) {
            todo.done = done;
        }
    }

    /// Changes a task's text; clearing it removes the task.
    pub fn edit(&mut self, id: u64, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            self.remove(id);
        } else if let Some(todo) = self.items.iter_mut().find(|t| t.id == id) {
            todo.text = text.to_string();
        }
    }

    pub fn remove(&mut self, id: u64) {
        self.items.retain(|t| t.id != id);
    }

    pub fn clear_done(&mut self) {
        self.items.retain(|t| !t.done);
    }

    pub fn google_id(&self, id: u64) -> Option<String> {
        self.items.iter().find(|t| t.id == id)?.google_id.clone()
    }

    /// A task now has its Google id; a copy that came in meanwhile is dropped.
    pub fn set_google_id(&mut self, id: u64, google_id: String) {
        self.items
            .retain(|t| t.id == id || t.google_id.as_deref() != Some(&google_id));
        if let Some(todo) = self.items.iter_mut().find(|t| t.id == id) {
            todo.google_id = Some(google_id);
        }
    }

    /// Takes Google's open tasks as they are, keeping what Google can't know yet:
    /// tasks not uploaded yet, and ticked ones still leaving.
    pub fn merge_remote(&mut self, remote: Vec<(String, String)>) {
        let mut merged: Vec<Todo> = self
            .items
            .iter()
            .filter(|t| t.google_id.is_none())
            .cloned()
            .collect();
        for (google_id, text) in remote {
            let local = self
                .items
                .iter()
                .find(|t| t.google_id.as_deref() == Some(&google_id));
            let id = match local {
                Some(todo) => todo.id,
                None => {
                    self.next_id += 1;
                    self.next_id - 1
                }
            };
            merged.push(Todo {
                id,
                text,
                done: local.is_some_and(|t| t.done),
                google_id: Some(google_id),
            });
        }
        // Ticked tasks Google no longer lists stay until their animation ends.
        for (index, todo) in self.items.iter().enumerate() {
            if todo.done && !merged.iter().any(|t| t.id == todo.id) {
                merged.insert(index.min(merged.len()), todo.clone());
            }
        }
        self.items = merged;
    }

    pub fn open_count(&self) -> usize {
        self.items.iter().filter(|t| !t.done).count()
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(todos: &Todos) -> Vec<(&str, bool)> {
        todos.items.iter().map(|t| (t.text.as_str(), t.done)).collect()
    }

    #[test]
    fn order_of_open_and_done() {
        let mut todos = Todos::default();
        let a = todos.add("a").unwrap();
        let b = todos.add(" b ").unwrap();
        let c = todos.add("c").unwrap();
        assert_eq!(todos.add("   "), None);
        assert_eq!(texts(&todos), [("c", false), ("b", false), ("a", false)]);

        todos.set_done(c, true);
        todos.set_done(a, true);
        assert_eq!(texts(&todos), [("c", true), ("b", false), ("a", true)]);
        assert_eq!(todos.open_count(), 1);

        todos.set_done(c, false);
        todos.edit(b, "bb");
        todos.edit(c, "");
        assert_eq!(texts(&todos), [("bb", false), ("a", true)]);

        todos.clear_done();
        assert_eq!(texts(&todos), [("bb", false)]);
    }

    #[test]
    fn stored_tasks_keep_ids_unique() {
        let mut todos = Todos::from_stored(vec![
            Todo { id: 4, text: "done".into(), done: true, google_id: None },
            Todo { id: 7, text: "open".into(), done: false, google_id: None },
        ]);
        assert_eq!(texts(&todos), [("open", false)]);
        assert_eq!(todos.add("new"), Some(8));
    }

    #[test]
    fn merging_google_tasks() {
        let mut todos = Todos::default();
        let local = todos.add("jen tady").unwrap();
        let synced = todos.add("synced").unwrap();
        todos.set_google_id(synced, "g1".into());
        let ticked = todos.add("odškrtnutý").unwrap();
        todos.set_google_id(ticked, "g2".into());
        todos.set_done(ticked, true);

        todos.merge_remote(vec![
            ("g3".into(), "z Googlu".into()),
            ("g1".into(), "přejmenovaný".into()),
        ]);
        assert_eq!(
            texts(&todos),
            [("odškrtnutý", true), ("jen tady", false), ("z Googlu", false), ("přejmenovaný", false)]
        );
        // Ids of known tasks stay, new ones get fresh ids.
        assert_eq!(todos.items[3].id, synced);
        assert!(todos.items[2].id > ticked);
        assert_eq!(todos.items[1].id, local);

        // The upload of a local task finished after Google already listed it.
        todos.merge_remote(vec![("g4".into(), "jen tady".into())]);
        todos.set_google_id(local, "g4".into());
        assert_eq!(
            todos.items.iter().filter(|t| t.google_id.as_deref() == Some("g4")).count(),
            1
        );
    }
}
