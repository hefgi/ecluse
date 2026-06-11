use anyhow::Result;

use crate::config::Config;
use crate::state::State;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, HookConfig, Mode};
    use crate::state::{Session, State};

    fn make_config(max_slots: u8) -> Config {
        Config {
            mode: Mode::Host,
            max_slots,
            prefix: "ecluse".into(),
            worktree_dir: ".ecluse/worktrees".into(),
            app_label: "ecluse.role".into(),
            app_label_value: "app".into(),
            strict_port: false,
            port_search_range: 10,
            slot_stride: 1,
            services: vec![],
            hooks: HookConfig::default(),
            inherit_env: vec![],
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
                    branch: format!("branch-{}", slot),
                    worktree_path: format!("/tmp/wt-{}", slot),
                    status: crate::state::SessionStatus::Active,
                    pending_op: None,
                    compose_project: None,
                    overlay_file: None,
                    overlay_files: vec![],
                    compose_overlays: vec![],
                    app_port: None,
                    started_at: "2026-01-01T00:00:00Z".into(),
                    port_overrides: std::collections::HashMap::new(),
                    process_manager: None,
                    tmux_session: None,
                    pid_files: vec![],
                    log_dir: None,
                    services_subset: None,
                })
                .collect(),
        }
    }

    #[test]
    fn allocates_first_slot_when_empty() {
        let config = make_config(8);
        let state = make_state(&[]);
        let allocator = SlotAllocator::new(&config, &state);
        assert_eq!(allocator.allocate_next().unwrap(), 1);
    }

    #[test]
    fn allocates_next_free_slot() {
        let config = make_config(8);
        let state = make_state(&[1, 2, 4]);
        let allocator = SlotAllocator::new(&config, &state);
        assert_eq!(allocator.allocate_next().unwrap(), 3);
    }

    #[test]
    fn fails_when_all_slots_full() {
        let config = make_config(3);
        let state = make_state(&[1, 2, 3]);
        let allocator = SlotAllocator::new(&config, &state);
        let err = allocator.allocate_next().unwrap_err();
        assert!(err.to_string().contains("3 slots are in use"));
    }

    #[test]
    fn allocates_slot_1_after_slots_2_and_3_used() {
        let config = make_config(8);
        let state = make_state(&[2, 3]);
        let allocator = SlotAllocator::new(&config, &state);
        assert_eq!(allocator.allocate_next().unwrap(), 1);
    }

    #[test]
    fn allocates_last_slot_when_all_before_used() {
        let config = make_config(4);
        let state = make_state(&[1, 2, 3]);
        let allocator = SlotAllocator::new(&config, &state);
        assert_eq!(allocator.allocate_next().unwrap(), 4);
    }

    #[test]
    fn exhausted_error_message_contains_max_slots() {
        let config = make_config(2);
        let state = make_state(&[1, 2]);
        let allocator = SlotAllocator::new(&config, &state);
        let err = allocator.allocate_next().unwrap_err();
        assert!(
            err.to_string().contains("2 slots are in use"),
            "got: {}",
            err
        );
    }

    #[test]
    fn max_slots_one_allocates_correctly() {
        let config = make_config(1);
        let state = make_state(&[]);
        let allocator = SlotAllocator::new(&config, &state);
        assert_eq!(allocator.allocate_next().unwrap(), 1);
    }

    #[test]
    fn max_slots_one_exhausted_after_one_session() {
        let config = make_config(1);
        let state = make_state(&[1]);
        let allocator = SlotAllocator::new(&config, &state);
        assert!(allocator.allocate_next().is_err());
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
}
