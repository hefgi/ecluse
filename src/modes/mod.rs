pub mod container;
pub mod host;
pub mod hybrid;

use anyhow::{Context, Result};
use indexmap::IndexMap;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::compose;
use crate::config::{Config, Mode, ServiceConfig};
use crate::docker;
use crate::log::StepLogger;
use crate::process::{ProcessManager, SpawnResult};
use crate::rollback::Rollback;
use crate::state::Session;
use crate::worktree::WorktreeManager;

/// Everything `bring_up` needs beyond config/root/log, bundled so the trait
/// signature stays stable as options are added.
pub struct BringUpRequest<'a> {
    pub slug: &'a str,
    pub slot: u8,
    pub branch: &'a str,
    pub watch: bool,
    pub reuse_worktree: bool,
    pub no_inherit_env: bool,
    pub worktree_override: Option<PathBuf>,
    /// Explicit --port name=value pins.
    pub port_overrides: &'a HashMap<String, u16>,
    /// --services subset; None means all services.
    pub service_filter: Option<&'a HashSet<String>>,
    /// Services to leave untouched (already running, or --skip).
    pub skip_services: &'a HashSet<String>,
    /// Ports recorded for the existing session when resuming.
    pub existing_port_overrides: &'a HashMap<String, u16>,
}

pub trait ModeHandler {
    fn bring_up(
        &self,
        req: &BringUpRequest,
        config: &Config,
        root: &Path,
        log: &StepLogger,
    ) -> Result<Session>;

    fn bring_down(
        &self,
        session: &Session,
        config: &Config,
        root: &Path,
        keep_volumes: bool,
        keep_worktree: bool,
        log: &StepLogger,
    ) -> Result<()>;
}

pub fn get_handler(config: &Config) -> Box<dyn ModeHandler> {
    get_handler_for_mode(&config.mode)
}

/// Dispatch on an explicit mode. Teardown paths must use the mode recorded in
/// the session, not the config's current mode — `.ecluse.toml` may have changed
/// since `up` (e.g. hybrid → host would otherwise strand the session's containers).
pub fn get_handler_for_mode(mode: &Mode) -> Box<dyn ModeHandler> {
    match mode {
        Mode::Container => Box::new(container::ContainerMode),
        Mode::Host => Box::new(host::HostMode),
        Mode::Hybrid => Box::new(hybrid::HybridMode),
    }
}

// ── Shared bring_up building blocks ───────────────────────────────────────────

/// The docker `[[services]]` selected by the request's --services filter.
pub(crate) fn filtered_docker_services<'c>(
    config: &'c Config,
    req: &BringUpRequest,
) -> Vec<&'c ServiceConfig> {
    config
        .docker_services()
        .into_iter()
        .filter(|s| req.service_filter.is_none_or(|f| f.contains(&s.name)))
        .collect()
}

/// What `start_docker_services` brought up (or copied from the existing session).
#[derive(Default)]
pub(crate) struct DockerStartup {
    pub allocated_ports: Vec<(String, u16)>,
    pub written_overlays: Vec<String>,
    pub compose_overlays: Vec<crate::state::ComposeOverlay>,
}

/// Bring up the `[[services]]`-declared docker services, one compose group at a
/// time, registering an undo with `rollback` after each successful step.
/// `limit_to_listed` scopes the overlay and `compose up` to the listed services
/// (hybrid); container mode brings up whole compose files.
pub(crate) fn start_docker_services(
    req: &BringUpRequest,
    config: &Config,
    root: &Path,
    limit_to_listed: bool,
    rollback: &mut Rollback,
    log: &StepLogger,
) -> Result<DockerStartup> {
    let project = compose_project_name(config, req.slug);
    let overlay_dir = root.join(".ecluse").join("overlays");
    std::fs::create_dir_all(&overlay_dir).context("failed to create overlays directory")?;
    // Only delete volumes the rollback created: on resume the session's
    // existing data volumes must survive a failed re-up.
    let rollback_volumes = !req.reuse_worktree;

    let docker_svcs_config = filtered_docker_services(config, req);
    let mut out = DockerStartup::default();

    // Copy ports for skipped docker services from the existing session.
    for svc in &docker_svcs_config {
        if req.skip_services.contains(&svc.name) {
            if let Some(&p) = req.existing_port_overrides.get(&svc.name) {
                log.detail(&format!("{}: already running — skipped", svc.name));
                out.allocated_ports.push((svc.name.clone(), p));
            }
        }
    }

    let docker_svcs_to_start: Vec<_> = docker_svcs_config
        .iter()
        .filter(|s| !req.skip_services.contains(&s.name))
        .copied()
        .collect();

    if docker_svcs_to_start.is_empty() {
        return Ok(out);
    }

    let groups = group_by_compose(root, &docker_svcs_to_start)?;
    for (compose_path, svcs) in &groups {
        let svc_names: Vec<String> = svcs.iter().map(|s| s.name.clone()).collect();
        log.step(&format!(
            "Starting docker services: {}...",
            svc_names.join(", ")
        ));

        let compose_data = compose::parse(compose_path)?;

        // Build port_map: services that publish their primary base_port to the host.
        // Services with suppress_primary_publish are excluded — their only host-side
        // publish is via extra_port_map.
        let mut port_map: HashMap<String, (u16, u16)> = HashMap::new();
        for s in svcs {
            if s.suppress_primary_publish() {
                // Track the first extra_port host port as the "primary" for state/ls.
                if let Some(ep) = s.extra_ports.first() {
                    let hp = ep.base_port.saturating_add(req.slot as u16);
                    out.allocated_ports.push((s.name.clone(), hp));
                    log.detail(&format!("{}: {hp} (via extra_ports)", s.name));
                }
            } else {
                let host_port = if let Some(&p) = req.port_overrides.get(&s.name) {
                    p
                } else {
                    crate::validate::find_free_port(config, s, req.slot)?
                };
                out.allocated_ports.push((s.name.clone(), host_port));
                log.detail(&format!("{}: {host_port}", s.name));
                port_map.insert(s.name.clone(), (host_port, s.base_port));
            }
        }

        let overlay_name = overlay_name_for_compose(req.slug, compose_path, root);
        let overlay_path = overlay_dir.join(&overlay_name);

        // Build extra_port_map using container_port from ExtraPort when set.
        let extra_port_map: HashMap<String, Vec<(u16, u16)>> = svcs
            .iter()
            .filter_map(|s| {
                let extras: Vec<(u16, u16)> = s
                    .extra_port_mappings()
                    .into_iter()
                    .map(|(host_base, cport)| (host_base.saturating_add(req.slot as u16), cport))
                    .collect();
                if extras.is_empty() {
                    None
                } else {
                    Some((s.name.clone(), extras))
                }
            })
            .collect();

        // Env map for compose interpolation: ECLUSE_<NAME>_PORT + extra_ports vars.
        let mut compose_env: HashMap<String, String> = port_map
            .iter()
            .map(|(n, (hp, _))| (format!("ECLUSE_{}_PORT", n.to_uppercase()), hp.to_string()))
            .collect();
        for svc in svcs {
            for ep in &svc.extra_ports {
                let host_port = ep.base_port.saturating_add(req.slot as u16);
                compose_env.insert(ep.port_env.clone(), host_port.to_string());
            }
        }

        let scope: Option<&[String]> = if limit_to_listed {
            Some(&svc_names)
        } else {
            None
        };
        let yaml = compose::generate_overlay_with_ports(
            &compose_data,
            &port_map,
            &extra_port_map,
            &project,
            scope,
            &config.prefix,
            req.slot,
        )?;
        std::fs::write(&overlay_path, &yaml).context("failed to write overlay file")?;
        {
            let overlay = overlay_path.clone();
            rollback.push(move || {
                let _ = std::fs::remove_file(&overlay);
            });
        }

        let compose_str = compose_path.to_string_lossy().to_string();
        let overlay_str = overlay_path.to_string_lossy().to_string();

        if limit_to_listed {
            let svc_refs: Vec<&str> = svc_names.iter().map(|s| s.as_str()).collect();
            docker::compose_up_services(
                &project,
                &compose_str,
                Some(&overlay_str),
                &svc_refs,
                req.watch,
                &compose_env,
            )?;
        } else {
            docker::compose_up(
                &project,
                &compose_str,
                Some(&overlay_str),
                req.watch,
                &compose_env,
            )?;
        }
        {
            let (p, c, o) = (project.clone(), compose_str.clone(), overlay_str.clone());
            rollback.push(move || {
                let _ = docker::compose_down(&p, &c, Some(&o), rollback_volumes);
            });
        }

        out.compose_overlays.push(crate::state::ComposeOverlay {
            compose: compose_str,
            overlay: overlay_str.clone(),
        });
        out.written_overlays.push(overlay_str);
    }

    Ok(out)
}

pub(crate) fn compose_project_name(config: &Config, slug: &str) -> String {
    format!("{}_{}", config.prefix, slug)
}

/// Resolve and (unless reusing) create the session worktree, registering its
/// removal with `rollback`.
pub(crate) fn ensure_worktree(
    req: &BringUpRequest,
    config: &Config,
    root: &Path,
    rollback: &mut Rollback,
    log: &StepLogger,
) -> Result<PathBuf> {
    let wt = WorktreeManager::new(root.to_owned());
    let worktree_path = req
        .worktree_override
        .clone()
        .unwrap_or_else(|| wt.worktree_path(config, req.slug));

    if req.reuse_worktree {
        if !worktree_path.exists() {
            return Err(anyhow::anyhow!(
                "worktree not found at {}; remove --reuse-worktree or run ecluse up without it",
                worktree_path.display()
            ));
        }
        log.step("Reusing existing worktree...");
        log.detail(&worktree_path.display().to_string());
    } else {
        log.step(&format!("Creating worktree (branch: {})...", req.branch));
        log.detail(&worktree_path.display().to_string());
        wt.create(&worktree_path, req.branch)?;
        {
            let root_owned = root.to_owned();
            let wt_path = worktree_path.clone();
            rollback.push(move || {
                let _ = WorktreeManager::new(root_owned).remove(&wt_path);
            });
        }
    }
    Ok(worktree_path)
}

/// Build the native port map for a slot, falling back to "app" on 3000+slot
/// when no native `[[services]]` match. Skipped services copy their port from
/// `existing` instead of probing.
///
/// `filter` limits which services get ports (hybrid honors --services here;
/// host historically allocates all native ports regardless — pass None there).
pub(crate) fn native_ports_for_slot(
    config: &Config,
    slot: u8,
    overrides: &HashMap<String, u16>,
    skip: &HashSet<String>,
    existing: &HashMap<String, u16>,
    filter: Option<&HashSet<String>>,
) -> Result<IndexMap<String, u16>> {
    let native: Vec<&ServiceConfig> = config
        .native_services()
        .into_iter()
        .filter(|s| filter.is_none_or(|f| f.contains(&s.name)))
        .collect();
    if native.is_empty() {
        let port = if let Some(&p) = overrides.get("app").or_else(|| existing.get("app")) {
            p
        } else {
            let fallback = ServiceConfig {
                name: "app".into(),
                base_port: 3000,
                run: crate::config::ServiceRun::Native,
                compose: None,
                command: None,
                port_env: vec![],
                debug_port: None,
                extra_ports: vec![],
                host_port: None,
            };
            crate::validate::find_free_port(config, &fallback, slot)?
        };
        let mut m = IndexMap::new();
        m.insert("app".to_string(), port);
        Ok(m)
    } else {
        native
            .iter()
            .map(|s| {
                let port = if let Some(&p) = overrides.get(&s.name) {
                    p
                } else if skip.contains(&s.name) {
                    existing.get(&s.name).copied().ok_or_else(|| {
                        anyhow::anyhow!(
                            "service '{}' is skipped but has no recorded port; run ecluse up without --skip or provide --port {}=<value>",
                            s.name, s.name
                        )
                    })?
                } else {
                    crate::validate::find_free_port(config, s, slot)?
                };
                Ok((s.name.clone(), port))
            })
            .collect()
    }
}

/// Spawn the non-skipped native services with the configured process manager,
/// registering a kill with `rollback`. Returns the spawn result and the
/// manager that was used.
pub(crate) fn spawn_native_services(
    req: &BringUpRequest,
    native_svcs: &[&ServiceConfig],
    native_ports: &IndexMap<String, u16>,
    worktree_path: &Path,
    env_map: &HashMap<String, String>,
    rollback: &mut Rollback,
    log: &StepLogger,
) -> Result<(SpawnResult, ProcessManager)> {
    let global = crate::process::load_global_config()?;

    let svcs_to_spawn: Vec<&ServiceConfig> = native_svcs
        .iter()
        .filter(|s| !req.skip_services.contains(&s.name))
        .copied()
        .collect();

    if svcs_to_spawn.iter().any(|s| s.command.is_some()) {
        log.step(&format!(
            "Spawning native services ({})...",
            global.process_manager
        ));
        for svc in &svcs_to_spawn {
            if let Some(cmd) = &svc.command {
                let port = native_ports.get(&svc.name).copied().unwrap_or(0);
                log.detail(&format!("{} on port {} — {}", svc.name, port, cmd));
            }
        }
    }
    let spawn = crate::process::spawn_services(
        &global.process_manager,
        req.slug,
        &svcs_to_spawn,
        worktree_path,
        env_map,
    )?;
    if spawn.tmux_session.is_some() || !spawn.pid_files.is_empty() {
        let manager = global.process_manager.clone();
        let spawned = spawn.clone();
        rollback.push(move || crate::process::kill_services(&manager, &spawned));
    }
    Ok((spawn, global.process_manager))
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

/// Legacy teardown for state files that predate `Session.compose_overlays`:
/// reconstructs each overlay's compose file from the overlay *filename*,
/// falling back to the root compose. Filename reconstruction is ambiguous for
/// hyphenated slugs (see `compose_file_for_overlay`) — sessions written by
/// current versions carry explicit pairs and never go through this path.
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
            slot_stride: 1,
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
            extra_ports: vec![],
            host_port: None,
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

    #[test]
    fn get_handler_for_mode_returns_handler_for_each_mode() {
        let _ = get_handler_for_mode(&Mode::Container);
        let _ = get_handler_for_mode(&Mode::Host);
        let _ = get_handler_for_mode(&Mode::Hybrid);
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
