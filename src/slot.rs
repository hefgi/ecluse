use anyhow::Result;

use crate::config::Config;
use crate::state::State;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, DatabaseConfig, Mode};
    use crate::state::{Session, State};

    fn make_config(max_slots: u8, stride: u16) -> Config {
        Config {
            mode: Mode::Host,
            max_slots,
            base_port: 3000,
            stride,
            prefix: "ecluse".into(),
            worktree_dir: ".ecluse/worktrees".into(),
            app_label: "ecluse.role".into(),
            app_label_value: "app".into(),
            database: DatabaseConfig::default(),
        }
    }

    fn make_state(slots: &[u8]) -> State {
        State {
            version: 1,
            sessions: slots
                .iter()
                .map(|&slot| Session {
                    slug: format!("sess-{}", slot),
                    mode: Mode::Host,
                    slot,
                    offset: slot as u16 * 100,
                    branch: format!("branch-{}", slot),
                    worktree_path: format!("/tmp/wt-{}", slot),
                    compose_project: None,
                    overlay_file: None,
                    app_port: None,
                    database_name: None,
                    started_at: "2026-01-01T00:00:00Z".into(),
                })
                .collect(),
        }
    }

    #[test]
    fn allocates_first_slot_when_empty() {
        let config = make_config(8, 100);
        let state = make_state(&[]);
        let allocator = SlotAllocator::new(&config, &state);
        assert_eq!(allocator.allocate_next().unwrap(), 1);
    }

    #[test]
    fn allocates_next_free_slot() {
        let config = make_config(8, 100);
        let state = make_state(&[1, 2, 4]);
        let allocator = SlotAllocator::new(&config, &state);
        assert_eq!(allocator.allocate_next().unwrap(), 3);
    }

    #[test]
    fn fails_when_all_slots_full() {
        let config = make_config(3, 100);
        let state = make_state(&[1, 2, 3]);
        let allocator = SlotAllocator::new(&config, &state);
        let err = allocator.allocate_next().unwrap_err();
        assert!(err.to_string().contains("3 slots are in use"));
    }

    #[test]
    fn offset_is_slot_times_stride() {
        let config = make_config(8, 100);
        let state = make_state(&[]);
        let allocator = SlotAllocator::new(&config, &state);
        assert_eq!(allocator.offset(1), 100);
        assert_eq!(allocator.offset(3), 300);
        assert_eq!(allocator.offset(8), 800);
    }

    #[test]
    fn offset_respects_custom_stride() {
        let config = make_config(8, 50);
        let state = make_state(&[]);
        let allocator = SlotAllocator::new(&config, &state);
        assert_eq!(allocator.offset(2), 100);
        assert_eq!(allocator.offset(5), 250);
    }
}

pub struct SlotAllocator<'a> {
    config: &'a Config,
    state: &'a State,
}

impl<'a> SlotAllocator<'a> {
    pub fn new(config: &'a Config, state: &'a State) -> Self {
        Self { config, state }
    }

    pub fn allocate_next(&self) -> Result<u8> {
        let used = self.state.used_slots();
        for slot in 1..=self.config.max_slots {
            if !used.contains(&slot) {
                return Ok(slot);
            }
        }
        Err(crate::error::EcluseError::SlotsExhausted(self.config.max_slots).into())
    }

    pub fn offset(&self, slot: u8) -> u16 {
        slot as u16 * self.config.stride
    }
}
