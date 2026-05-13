use anyhow::Result;

use crate::config::Config;
use crate::state::State;

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
