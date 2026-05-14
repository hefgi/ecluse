pub mod container;
pub mod host;
pub mod hybrid;

use anyhow::Result;
use indexmap::IndexMap;
use std::path::Path;

use crate::compose;
use crate::config::{Config, Mode, ServiceConfig};
use crate::docker;
use crate::state::Session;

pub trait ModeHandler {
    fn bring_up(
        &self,
        slug: &str,
        slot: u8,
        branch: &str,
        config: &Config,
        root: &Path,
        watch: bool,
        reuse_worktree: bool,
        port_overrides: &std::collections::HashMap<String, u16>,
    ) -> Result<Session>;

    fn bring_down(
        &self,
        session: &Session,
        config: &Config,
        root: &Path,
        keep_volumes: bool,
        keep_worktree: bool,
    ) -> Result<()>;
}

pub fn get_handler(config: &Config) -> Box<dyn ModeHandler> {
    match config.mode {
        Mode::Container => Box::new(container::ContainerMode),
        Mode::Host => Box::new(host::HostMode),
        Mode::Hybrid => Box::new(hybrid::HybridMode),
    }
}

// ── Shared helpers for multi-compose-file support ─────────────────────────────

/// Group docker services by the compose file they belong to.
/// Services without an explicit `compose` field all share the root compose file.
pub fn group_by_compose<'a>(
    root: &Path,
    svcs: &[&'a ServiceConfig],
) -> Result<Vec<(std::path::PathBuf, Vec<&'a ServiceConfig>)>> {
    let mut groups: IndexMap<std::path::PathBuf, Vec<&'a ServiceConfig>> = IndexMap::new();
    for svc in svcs {
        let compose_path = compose::resolve_service_compose(root, svc).ok_or_else(|| {
            anyhow::anyhow!(
                "compose file not found for service '{}' (compose = {:?})",
                svc.name,
                svc.compose
            )
        })?;
        groups.entry(compose_path).or_default().push(svc);
    }
    Ok(groups.into_iter().collect())
}

/// Derive an overlay filename for a given slug + compose file path.
/// Root compose → `<slug>.yml`. Per-service compose → `<slug>-<parent-dir>.yml`.
pub fn overlay_name_for_compose(slug: &str, compose_path: &Path, root: &Path) -> String {
    // Check if this IS the root compose file
    let is_root = compose::find_compose_file(root)
        .map(|p| p == compose_path)
        .unwrap_or(false);
    if is_root {
        return format!("{}.yml", slug);
    }
    let stem = compose_path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("extra");
    format!("{}-{}.yml", slug, stem)
}

/// Tear down all (compose, overlay) pairs. For each overlay, reconstruct the
/// compose file path from the overlay filename, falling back to the root compose.
pub fn tear_down_all_overlays(project: &str, root: &Path, overlays: &[String], remove_volumes: bool) {
    for overlay_str in overlays {
        let compose_path = compose_file_for_overlay(root, overlay_str)
            .or_else(|| compose::find_compose_file(root));
        if let Some(cp) = compose_path {
            let _ = docker::compose_down(project, &cp.to_string_lossy(), Some(overlay_str), remove_volumes);
        }
    }
}

/// Given an overlay path `…/<slug>-<stem>.yml`, look for a compose file in
/// `root/<stem>/`. Returns None for root overlays (`<slug>.yml`, no hyphen-stem).
fn compose_file_for_overlay(root: &Path, overlay_str: &str) -> Option<std::path::PathBuf> {
    let filename = std::path::Path::new(overlay_str).file_stem()?.to_str()?;
    let stem = filename.splitn(2, '-').nth(1)?;
    compose::find_compose_file(&root.join(stem))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{HookConfig, Mode};

    fn make_config(mode: Mode) -> Config {
        Config {
            mode,
            max_slots: 8,
            prefix: "ecluse".into(),
            worktree_dir: ".ecluse/worktrees".into(),
            app_label: "ecluse.role".into(),
            app_label_value: "app".into(),
            strict_port: false,
            port_search_range: 10,
            services: vec![],
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
