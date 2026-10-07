//! Persistent queue of files that failed to transfer and should be resent later.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FailedEntry {
    pub relative_path: String,
    pub absolute_path: String,
    pub size: u64,
    pub error: String,
    /// Unix epoch seconds.
    pub failed_at_unix: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RetryQueue {
    pub entries: Vec<FailedEntry>,
}

impl RetryQueue {
    pub fn load(path: &Path) -> Self {
        let Ok(bytes) = fs::read(path) else {
            return Self::default();
        };
        serde_json::from_slice(&bytes).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(Error::from_io)?;
        }
        let json = serde_json::to_vec_pretty(self)
            .map_err(|e| Error::Other(format!("retry queue serialize: {e}")))?;
        fs::write(path, json).map_err(Error::from_io)
    }

    pub fn merge(&mut self, incoming: Vec<FailedEntry>) {
        for entry in incoming {
            if let Some(existing) = self
                .entries
                .iter_mut()
                .find(|e| e.absolute_path == entry.absolute_path)
            {
                *existing = entry;
            } else {
                self.entries.push(entry);
            }
        }
    }

    pub fn remove_absolutes(&mut self, absolutes: &[String]) {
        self.entries
            .retain(|e| !absolutes.iter().any(|a| a == &e.absolute_path));
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn failed_entry(
    relative_path: impl Into<String>,
    absolute_path: impl Into<PathBuf>,
    size: u64,
    error: impl Into<String>,
) -> FailedEntry {
    FailedEntry {
        relative_path: relative_path.into(),
        absolute_path: absolute_path.into().to_string_lossy().into_owned(),
        size,
        error: error.into(),
        failed_at_unix: now_unix(),
    }
}

/// Append failures into the on-disk queue (create/merge/save).
pub fn record_failures(path: &Path, failures: Vec<FailedEntry>) -> Result<()> {
    if failures.is_empty() {
        return Ok(());
    }
    let mut queue = RetryQueue::load(path);
    queue.merge(failures);
    queue.save(path)
}

/// Remove successfully resent files from the queue.
pub fn mark_sent(path: &Path, absolutes: &[String]) -> Result<()> {
    if absolutes.is_empty() || !path.exists() {
        return Ok(());
    }
    let mut queue = RetryQueue::load(path);
    queue.remove_absolutes(absolutes);
    queue.save(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn merge_and_persist() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("retry.json");
        let a = failed_entry("a.txt", dir.path().join("a.txt"), 1, "boom");
        record_failures(&path, vec![a.clone()]).unwrap();
        let b = failed_entry("b.txt", dir.path().join("b.txt"), 2, "nope");
        record_failures(&path, vec![b.clone()]).unwrap();
        let q = RetryQueue::load(&path);
        assert_eq!(q.len(), 2);
        mark_sent(&path, &[a.absolute_path.clone()]).unwrap();
        let q = RetryQueue::load(&path);
        assert_eq!(q.len(), 1);
        assert_eq!(q.entries[0].relative_path, "b.txt");
    }
}
