pub mod container;
pub mod host;
pub mod hybrid;

use anyhow::Result;
use std::path::Path;

use crate::config::{Config, Mode};
use crate::state::Session;

pub trait ModeHandler {
    #[allow(clippy::too_many_arguments)]
    fn bring_up(
        &self,
        slug: &str,
        slot: u8,
        offset: u16,
        branch: &str,
        config: &Config,
        root: &Path,
        watch: bool,
    ) -> Result<Session>;

    fn bring_down(
        &self,
        session: &Session,
        config: &Config,
        root: &Path,
        keep_volumes: bool,
        keep_database: bool,
    ) -> Result<()>;
}

pub fn get_handler(config: &Config) -> Box<dyn ModeHandler> {
    match config.mode {
        Mode::Container => Box::new(container::ContainerMode),
        Mode::Host => Box::new(host::HostMode),
        Mode::Hybrid => Box::new(hybrid::HybridMode),
    }
}
