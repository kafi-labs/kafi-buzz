//! Persistent session mapping state (atomic JSON file).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::AdapterError;

/// Maximum mapped sessions retained (LRU by `last_used_at`).
pub const MAX_SESSIONS: usize = 200;

/// One channel → intel session mapping.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEntry {
    /// Intel session UUID.
    pub session_id: String,
    /// Entity id used when the session was created.
    pub entity_id: String,
    /// When the mapping was first created (RFC3339).
    pub created_at: DateTime<Utc>,
    /// Last successful use (for LRU pruning).
    #[serde(default = "Utc::now")]
    pub last_used_at: DateTime<Utc>,
    /// Whether the harness systemPrompt has already been forwarded.
    #[serde(default)]
    pub system_prompt_forwarded: bool,
}

/// On-disk state file body.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StateFile {
    /// Resolved intel agent UUID (cached after initialize).
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Keyed by Buzz channel UUID (or ACP session id when mode=acp).
    #[serde(default)]
    pub sessions: HashMap<String, SessionEntry>,
}

/// In-memory state with path for persistence.
#[derive(Debug)]
pub struct StateStore {
    path: PathBuf,
    data: StateFile,
}

impl StateStore {
    /// Load from disk (or start empty if missing).
    pub fn load(path: &Path) -> Result<Self, AdapterError> {
        let data = if path.exists() {
            let raw = std::fs::read_to_string(path)?;
            match serde_json::from_str(&raw) {
                Ok(d) => d,
                Err(e) => {
                    tracing::warn!(
                        "state: failed to parse {}: {e}; starting fresh",
                        path.display()
                    );
                    StateFile::default()
                }
            }
        } else {
            StateFile::default()
        };
        Ok(Self {
            path: path.to_path_buf(),
            data,
        })
    }

    /// Cached intel agent id.
    pub fn agent_id(&self) -> Option<&str> {
        self.data.agent_id.as_deref()
    }

    /// Persist resolved agent id.
    pub fn set_agent_id(&mut self, id: String) -> Result<(), AdapterError> {
        self.data.agent_id = Some(id);
        self.flush()
    }

    /// Lookup a session mapping.
    pub fn get_session(&self, key: &str) -> Option<&SessionEntry> {
        self.data.sessions.get(key)
    }

    /// Insert/update a session mapping and flush.
    pub fn put_session(&mut self, key: String, entry: SessionEntry) -> Result<(), AdapterError> {
        self.data.sessions.insert(key, entry);
        self.prune_lru();
        self.flush()
    }

    /// Touch last_used_at for LRU and optionally mark system prompt forwarded.
    pub fn touch_session(
        &mut self,
        key: &str,
        mark_system_prompt_forwarded: bool,
    ) -> Result<(), AdapterError> {
        if let Some(e) = self.data.sessions.get_mut(key) {
            e.last_used_at = Utc::now();
            if mark_system_prompt_forwarded {
                e.system_prompt_forwarded = true;
            }
            self.flush()?;
        }
        Ok(())
    }

    /// Drop a mapping (e.g. after 404/409).
    pub fn remove_session(&mut self, key: &str) -> Result<(), AdapterError> {
        self.data.sessions.remove(key);
        self.flush()
    }

    /// Write state atomically (tmp + rename).
    pub fn flush(&self) -> Result<(), AdapterError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(&self.data)?;
        std::fs::write(&tmp, json.as_bytes())?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    fn prune_lru(&mut self) {
        if self.data.sessions.len() <= MAX_SESSIONS {
            return;
        }
        let mut entries: Vec<(String, DateTime<Utc>)> = self
            .data
            .sessions
            .iter()
            .map(|(k, v)| (k.clone(), v.last_used_at))
            .collect();
        entries.sort_by_key(|(_, t)| *t);
        let excess = self.data.sessions.len() - MAX_SESSIONS;
        for (k, _) in entries.into_iter().take(excess) {
            self.data.sessions.remove(&k);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn round_trip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.json");
        let mut store = StateStore::load(&path).unwrap();
        store.set_agent_id("agent-1".into()).unwrap();
        store
            .put_session(
                "ch-1".into(),
                SessionEntry {
                    session_id: "sess-1".into(),
                    entity_id: "buzz:channel:ch-1".into(),
                    created_at: Utc::now(),
                    last_used_at: Utc::now(),
                    system_prompt_forwarded: false,
                },
            )
            .unwrap();

        let reloaded = StateStore::load(&path).unwrap();
        assert_eq!(reloaded.agent_id(), Some("agent-1"));
        assert_eq!(reloaded.get_session("ch-1").unwrap().session_id, "sess-1");
    }

    #[test]
    fn lru_prunes_beyond_cap() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.json");
        let mut store = StateStore::load(&path).unwrap();
        for i in 0..(MAX_SESSIONS + 5) {
            store
                .put_session(
                    format!("k{i}"),
                    SessionEntry {
                        session_id: format!("s{i}"),
                        entity_id: format!("e{i}"),
                        created_at: Utc::now(),
                        last_used_at: Utc::now() + chrono::Duration::seconds(i as i64),
                        system_prompt_forwarded: false,
                    },
                )
                .unwrap();
        }
        assert!(store.data.sessions.len() <= MAX_SESSIONS);
    }
}
