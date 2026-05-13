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
    ) -> Result<()>;
}

pub fn get_handler(config: &Config) -> Box<dyn ModeHandler> {
    match config.mode {
        Mode::Container => Box::new(container::ContainerMode),
        Mode::Host => Box::new(host::HostMode),
        Mode::Hybrid => Box::new(hybrid::HybridMode),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{HookConfig, Mode};

    fn make_config(mode: Mode) -> Config {
        Config {
            mode,
            max_slots: 8,
            base_port: 3000,
            stride: 100,
            prefix: "ecluse".into(),
            worktree_dir: ".ecluse/worktrees".into(),
            app_label: "ecluse.role".into(),
            app_label_value: "app".into(),
            ports: Default::default(),
            hooks: HookConfig::default(),
        }
    }

    #[test]
    fn get_handler_returns_handler_for_each_mode() {
        // Just verify no panic and handler is returned (can't easily inspect type)
        let _ = get_handler(&make_config(Mode::Container));
        let _ = get_handler(&make_config(Mode::Host));
        let _ = get_handler(&make_config(Mode::Hybrid));
    }
}
