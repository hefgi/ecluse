use anyhow::{Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config::Mode;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct State {
    #[serde(default = "default_version")]
    pub version: u8,
    #[serde(default)]
    pub sessions: Vec<Session>,
}

impl Default for State {
    fn default() -> Self {
        Self { version: 1, sessions: Vec::new() }
    }
}

fn default_version() -> u8 { 1 }

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Session {
    pub slug: String,
    pub mode: Mode,
    pub slot: u8,
    pub offset: u16,
    pub branch: String,
    pub worktree_path: String,
    pub compose_project: Option<String>,
    pub overlay_file: Option<String>,
    pub app_port: Option<u16>,
    pub database_name: Option<String>,
    pub started_at: String,
}

impl State {
    pub fn find_session(&self, slug: &str) -> Option<&Session> {
        self.sessions.iter().find(|s| s.slug == slug)
    }

    pub fn add_session(&mut self, session: Session) {
        self.sessions.push(session);
    }

    pub fn remove_session(&mut self, slug: &str) -> Option<Session> {
        if let Some(pos) = self.sessions.iter().position(|s| s.slug == slug) {
            Some(self.sessions.remove(pos))
        } else {
            None
        }
    }

    pub fn used_slots(&self) -> Vec<u8> {
        self.sessions.iter().map(|s| s.slot).collect()
    }
}

pub struct StateGuard {
    pub state: State,
    state_path: PathBuf,
    _lock_file: File,
}

impl StateGuard {
    pub fn acquire(root: &Path) -> Result<Self> {
        let ecluse_dir = root.join(".ecluse");
        std::fs::create_dir_all(&ecluse_dir)
            .with_context(|| format!("failed to create {}", ecluse_dir.display()))?;

        let lock_path = ecluse_dir.join("state.lock");
        let lock_file = OpenOptions::new()
            .create(true)
            .write(true)
            .open(&lock_path)
            .with_context(|| format!("failed to open lock file {}", lock_path.display()))?;

        // Try to acquire with a timeout by polling
        let start = std::time::Instant::now();
        loop {
            match lock_file.try_lock_exclusive() {
                Ok(()) => break,
                Err(_) => {
                    if start.elapsed() >= Duration::from_secs(10) {
                        return Err(crate::error::EcluseError::LockTimeout.into());
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }

        let state_path = ecluse_dir.join("state.json");
        let state = if state_path.exists() {
            let content = std::fs::read_to_string(&state_path)
                .context("failed to read state.json")?;
            serde_json::from_str(&content)
                .with_context(|| crate::error::EcluseError::StateCorrupt(content.clone()))?
        } else {
            State::default()
        };

        Ok(StateGuard {
            state,
            state_path,
            _lock_file: lock_file,
        })
    }

    pub fn commit(&self) -> Result<()> {
        let tmp = self.state_path.with_extension("json.tmp");
        let data = serde_json::to_string_pretty(&self.state)
            .context("failed to serialize state")?;
        std::fs::write(&tmp, &data)
            .context("failed to write state.json.tmp")?;
        std::fs::rename(&tmp, &self.state_path)
            .context("failed to atomically update state.json")?;
        Ok(())
    }
}
