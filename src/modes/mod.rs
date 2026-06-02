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
    #[allow(clippy::too_many_arguments)]
    fn bring_up(
        &self,
        slug: &str,
        slot: u8,
        branch: &str,
        config: &Config,
        root: &Path,
        watch: bool,
        reuse_worktree: bool,
        no_inherit_env: bool,
        worktree_override: Option<std::path::PathBuf>,
        port_overrides: &std::collections::HashMap<String, u16>,
        service_filter: Option<&std::collections::HashSet<String>>,
        skip_services: &std::collections::HashSet<String>,
        existing_port_overrides: &std::collections::HashMap<String, u16>,
        log: &crate::log::StepLogger,
    ) -> Result<Session>;

    fn bring_down(
        &self,
        session: &Session,
        config: &Config,
        root: &Path,
        keep_volumes: bool,
        keep_worktree: bool,
        log: &crate::log::StepLogger,
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
pub fn tear_down_all_overlays(
    project: &str,
    root: &Path,
    overlays: &[String],
    remove_volumes: bool,
) {
    for overlay_str in overlays {
        let compose_path = compose_file_for_overlay(root, overlay_str)
            .or_else(|| compose::find_compose_file(root));
        if let Some(cp) = compose_path {
            let _ = docker::compose_down(
                project,
                &cp.to_string_lossy(),
                Some(overlay_str),
                remove_volumes,
            );
        }
    }
}

/// Given an overlay path `…/<slug>-<stem>.yml`, look for a compose file in
/// `root/<stem>/`. Returns None for root overlays (`<slug>.yml`, no hyphen-stem).
/// Splits on the LAST hyphen so slugs like `feat-foo` don't consume the stem.
fn compose_file_for_overlay(root: &Path, overlay_str: &str) -> Option<std::path::PathBuf> {
    let filename = std::path::Path::new(overlay_str).file_stem()?.to_str()?;
    // stem is the part after the last '-'; if no '-', this is a root overlay
    #[allow(clippy::needless_splitn)]
    let stem = filename.rsplitn(2, '-').next()?;
    // If stem == filename there was no hyphen — root overlay, no subdir
    if stem == filename {
        return None;
    }
    compose::find_compose_file(&root.join(stem))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{HookConfig, Mode, ServiceRun};
    use tempfile::TempDir;

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
            inherit_env: vec![],
        }
    }

    fn docker_svc(name: &str, base_port: u16, compose: Option<&str>) -> ServiceConfig {
        ServiceConfig {
            name: name.into(),
            base_port,
            run: ServiceRun::Docker,
            compose: compose.map(|s| s.to_string()),
            command: None,
            port_env: vec![],
            debug_port: None,
        }
    }

    fn write_compose(dir: &std::path::Path, name: &str) {
        std::fs::write(dir.join(name), "services:\n  db:\n    image: postgres:16\n").unwrap();
    }

    #[test]
    fn get_handler_returns_handler_for_each_mode() {
        let _ = get_handler(&make_config(Mode::Container));
        let _ = get_handler(&make_config(Mode::Host));
        let _ = get_handler(&make_config(Mode::Hybrid));
    }

    // ── group_by_compose ──────────────────────────────────────────────────────

    #[test]
    fn group_by_compose_all_root_is_one_group() {
        let dir = TempDir::new().unwrap();
        write_compose(dir.path(), "docker-compose.yml");

        let pg = docker_svc("postgres", 5432, None);
        let redis = docker_svc("redis", 6379, None);
        let svcs = vec![&pg, &redis];

        let groups = group_by_compose(dir.path(), &svcs).unwrap();
        assert_eq!(
            groups.len(),
            1,
            "both services share root compose → one group"
        );
        assert_eq!(groups[0].1.len(), 2);
    }

    #[test]
    fn group_by_compose_explicit_paths_form_separate_groups() {
        let dir = TempDir::new().unwrap();
        write_compose(dir.path(), "docker-compose.yml");
        // Sub-directory compose
        std::fs::create_dir(dir.path().join("worker")).unwrap();
        write_compose(&dir.path().join("worker"), "docker-compose.yml");

        let pg = docker_svc("postgres", 5432, None);
        let worker = docker_svc("worker", 6379, Some("worker/docker-compose.yml"));
        let svcs = vec![&pg, &worker];

        let groups = group_by_compose(dir.path(), &svcs).unwrap();
        assert_eq!(groups.len(), 2, "two different compose files → two groups");
    }

    #[test]
    fn group_by_compose_same_explicit_path_merges_into_one_group() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join("infra")).unwrap();
        write_compose(&dir.path().join("infra"), "docker-compose.yml");

        let pg = docker_svc("postgres", 5432, Some("infra/docker-compose.yml"));
        let redis = docker_svc("redis", 6379, Some("infra/docker-compose.yml"));
        let svcs = vec![&pg, &redis];

        let groups = group_by_compose(dir.path(), &svcs).unwrap();
        assert_eq!(groups.len(), 1, "same compose file → merged group");
        assert_eq!(groups[0].1.len(), 2);
    }

    #[test]
    fn group_by_compose_missing_explicit_path_is_error() {
        let dir = TempDir::new().unwrap();
        let svc = docker_svc("db", 5432, Some("nonexistent/docker-compose.yml"));
        let svcs = vec![&svc];
        assert!(group_by_compose(dir.path(), &svcs).is_err());
    }

    #[test]
    fn group_by_compose_missing_root_and_no_explicit_is_error() {
        let dir = TempDir::new().unwrap();
        // No compose file anywhere, no explicit path
        let svc = docker_svc("db", 5432, None);
        let svcs = vec![&svc];
        assert!(group_by_compose(dir.path(), &svcs).is_err());
    }

    #[test]
    fn group_by_compose_preserves_insertion_order() {
        let dir = TempDir::new().unwrap();
        write_compose(dir.path(), "docker-compose.yml");
        std::fs::create_dir(dir.path().join("svc")).unwrap();
        write_compose(&dir.path().join("svc"), "docker-compose.yml");

        let pg = docker_svc("postgres", 5432, None);
        let svc = docker_svc("queue", 6379, Some("svc/docker-compose.yml"));
        let svcs = vec![&pg, &svc];

        let groups = group_by_compose(dir.path(), &svcs).unwrap();
        // Root group is first (postgres was inserted first)
        assert_eq!(groups[0].1[0].name, "postgres");
        assert_eq!(groups[1].1[0].name, "queue");
    }

    // ── overlay_name_for_compose ──────────────────────────────────────────────

    #[test]
    fn overlay_name_root_compose_uses_slug_only() {
        let dir = TempDir::new().unwrap();
        write_compose(dir.path(), "docker-compose.yml");
        let compose_path = dir.path().join("docker-compose.yml");
        assert_eq!(
            overlay_name_for_compose("feat-foo", &compose_path, dir.path()),
            "feat-foo.yml"
        );
    }

    #[test]
    fn overlay_name_subdirectory_compose_includes_dir_stem() {
        let dir = TempDir::new().unwrap();
        write_compose(dir.path(), "docker-compose.yml"); // root exists
        std::fs::create_dir(dir.path().join("worker")).unwrap();
        let compose_path = dir.path().join("worker").join("docker-compose.yml");
        let name = overlay_name_for_compose("feat-foo", &compose_path, dir.path());
        assert_eq!(name, "feat-foo-worker.yml");
    }

    #[test]
    fn overlay_name_no_root_compose_uses_parent_dir() {
        let dir = TempDir::new().unwrap();
        // No root compose — all composes are in subdirs
        std::fs::create_dir(dir.path().join("infra")).unwrap();
        let compose_path = dir.path().join("infra").join("docker-compose.yml");
        let name = overlay_name_for_compose("my-slug", &compose_path, dir.path());
        assert_eq!(name, "my-slug-infra.yml");
    }

    // ── compose_file_for_overlay (via tear_down logic) ────────────────────────

    #[test]
    fn compose_file_for_overlay_root_overlay_returns_none() {
        let dir = TempDir::new().unwrap();
        // `<slug>.yml` has no hyphen-stem → function returns None
        let overlay = dir.path().join(".ecluse/overlays/feat-foo.yml");
        let result = compose_file_for_overlay(dir.path(), overlay.to_str().unwrap());
        assert!(result.is_none());
    }

    #[test]
    fn compose_file_for_overlay_sub_overlay_finds_compose() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join("worker")).unwrap();
        write_compose(&dir.path().join("worker"), "docker-compose.yml");

        let overlay = dir
            .path()
            .join(".ecluse/overlays/feat-foo-worker.yml")
            .to_string_lossy()
            .to_string();
        let result = compose_file_for_overlay(dir.path(), &overlay);
        assert!(result.is_some());
        assert!(result.unwrap().ends_with("worker/docker-compose.yml"));
    }
}
