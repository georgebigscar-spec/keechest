//! The decrypted vault contents. Every string is wiped when dropped.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// How many previous passwords an entry keeps.
pub const HISTORY_LEN: usize = 10;

/// Seconds since the Unix epoch.
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct Vault {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub entries: Vec<Entry>,
}

/// A login. `group` is a free-form folder name, empty for none.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct Entry {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub group: String,
    #[serde(default)]
    pub favorite: bool,
    #[serde(default)]
    pub created: u64,
    #[serde(default)]
    pub modified: u64,
    /// Older passwords, newest first.
    #[serde(default)]
    pub history: Vec<HistoryItem>,
}

#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct HistoryItem {
    #[serde(default)]
    pub password: String,
    /// When this password was replaced.
    #[serde(default)]
    pub replaced: u64,
}

// Fields default one by one: a container-level `serde(default)` would need
// struct update syntax, which a `Drop` type does not allow.
// No Debug on purpose: it would be too easy to log a password.

impl Entry {
    /// An empty entry with a fresh id and timestamps.
    pub fn new() -> Self {
        let t = now();
        let mut e = Self::default();
        e.id = crate::random::id();
        e.created = t;
        e.modified = t;
        e
    }

    /// Whether `query` (already lowercased) appears in any visible field.
    /// Passwords and notes are not searched.
    pub fn matches(&self, query: &str) -> bool {
        query.is_empty()
            || [&self.title, &self.username, &self.url, &self.group]
                .iter()
                .any(|f| f.to_lowercase().contains(query))
    }
}

impl Vault {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            entries: Vec::new(),
        }
    }

    pub fn get(&self, id: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// Adds `entry`, or replaces the entry with the same id. A replaced
    /// password goes to the history.
    pub fn upsert(&mut self, mut entry: Entry) {
        entry.modified = now();
        match self.entries.iter_mut().find(|e| e.id == entry.id) {
            Some(old) => {
                let mut history = std::mem::take(&mut old.history);
                if !old.password.is_empty() && old.password != entry.password {
                    history.insert(
                        0,
                        HistoryItem {
                            password: std::mem::take(&mut old.password),
                            replaced: entry.modified,
                        },
                    );
                    history.truncate(HISTORY_LEN);
                }
                entry.history = history;
                *old = entry;
            }
            None => self.entries.push(entry),
        }
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.id != id);
        self.entries.len() != before
    }

    /// The distinct non-empty group names, sorted.
    pub fn groups(&self) -> Vec<String> {
        let mut groups: Vec<String> = self
            .entries
            .iter()
            .filter(|e| !e.group.is_empty())
            .map(|e| e.group.clone())
            .collect();
        groups.sort_by_key(|g| g.to_lowercase());
        groups.dedup();
        groups
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(title: &str, password: &str) -> Entry {
        let mut e = Entry::new();
        e.title = title.into();
        e.password = password.into();
        e
    }

    // Entry implements Drop, so struct update syntax is out.
    fn with(e: &Entry, f: impl FnOnce(&mut Entry)) -> Entry {
        let mut e = e.clone();
        f(&mut e);
        e
    }

    #[test]
    fn upsert_adds_then_replaces_and_keeps_history() {
        let mut v = Vault::new("test");
        let e = entry("Mail", "one");
        let id = e.id.clone();
        v.upsert(e.clone());
        assert_eq!(v.entries.len(), 1);

        v.upsert(with(&e, |e| e.password = "two".into()));
        v.upsert(with(&e, |e| {
            e.password = "two".into();
            e.title = "Mail!".into();
        }));
        let got = v.get(&id).unwrap();
        assert_eq!(v.entries.len(), 1);
        assert_eq!(got.title, "Mail!");
        assert_eq!(got.password, "two");
        assert_eq!(got.history.len(), 1);
        assert_eq!(got.history[0].password, "one");
    }

    #[test]
    fn history_is_capped() {
        let mut v = Vault::new("test");
        let e = entry("x", "p0");
        for i in 1..=HISTORY_LEN + 5 {
            v.upsert(with(&e, |e| e.password = format!("p{i}")));
        }
        let got = &v.entries[0];
        assert_eq!(got.history.len(), HISTORY_LEN);
        assert_eq!(got.history[0].password, format!("p{}", HISTORY_LEN + 4));
    }

    #[test]
    fn search_groups_and_remove() {
        let mut v = Vault::new("test");
        let mut a = entry("GitHub", "x");
        a.group = "Work".into();
        let mut b = entry("Bank", "y");
        b.group = "home".into();
        b.username = "me@example.com".into();
        let mut c = entry("Forum", "z");
        c.group = "Work".into();
        let id = a.id.clone();
        v.upsert(a);
        v.upsert(b);
        v.upsert(c);
        assert_eq!(v.groups(), ["home", "Work"]);
        assert!(v.get(&id).unwrap().matches("hub"));
        assert!(v.entries[1].matches("example"));
        assert!(!v.entries[1].matches("y"), "passwords are not searched");
        assert!(v.remove(&id));
        assert!(!v.remove(&id));
        assert_eq!(v.entries.len(), 2);
    }

    #[test]
    fn unknown_and_missing_fields_are_tolerated() {
        let v: Vault =
            serde_json::from_str(r#"{"entries":[{"title":"t","future":1}],"x":2}"#).unwrap();
        assert_eq!(v.entries[0].title, "t");
        assert!(v.entries[0].history.is_empty());
    }
}
