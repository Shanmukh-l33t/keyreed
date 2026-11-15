use crate::types::WritingMode;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub timestamp_unix: u64,
    pub mode: WritingMode,
    pub app_name: String,
    pub raw_transcript: String,
    pub final_text: String,
    pub duration_ms: u64,
}

/// Optional, strictly user-controlled local transcription history (Master Prompt Section 10 & 12).
/// Disabled (`enabled = false`) by default for privacy. Never stores audio.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryStore {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_max_entries")]
    pub max_entries: usize,
    #[serde(default = "default_retention_hours")]
    pub retention_hours: u64,
    #[serde(default)]
    pub entries: Vec<HistoryEntry>,
}

fn default_max_entries() -> usize {
    100
}

fn default_retention_hours() -> u64 {
    24 * 7
}

impl Default for HistoryStore {
    fn default() -> Self {
        Self {
            enabled: false, // Privacy-first default: off unless user enables history.
            max_entries: default_max_entries(),
            retention_hours: default_retention_hours(),
            entries: Vec::new(),
        }
    }
}

impl HistoryStore {
    pub fn now_unix() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    pub fn prune_expired(&mut self, now_unix: u64) {
        if self.retention_hours > 0 {
            let max_age_secs = self.retention_hours.saturating_mul(3600);
            self.entries
                .retain(|e| now_unix.saturating_sub(e.timestamp_unix) <= max_age_secs);
        }
        if self.entries.len() > self.max_entries {
            let excess = self.entries.len() - self.max_entries;
            self.entries.drain(0..excess);
        }
    }

    pub fn record(&mut self, entry: HistoryEntry) {
        if !self.enabled {
            return;
        }
        let now = entry.timestamp_unix;
        self.entries.push(entry);
        self.prune_expired(now);
    }

    pub fn clear_all(&mut self) {
        self.entries.clear();
    }

    pub fn load_or_default(path: &Path) -> Self {
        match fs::read_to_string(path) {
            Ok(raw) => {
                let mut store: Self = serde_json::from_str(&raw).unwrap_or_default();
                store.prune_expired(Self::now_unix());
                store
            }
            Err(_) => Self::default(),
        }
    }

    pub fn save_atomic(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        let serialized = serde_json::to_string_pretty(self)?;
        fs::write(&tmp, serialized)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }
}
