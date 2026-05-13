use anyhow::{Context, Result};
use chrono::Utc;
use std::path::Path;

use crate::compose;
use crate::config::Config;
use crate::docker;
use crate::env;
use crate::postgres::PgClient;
use crate::state::Session;
use crate::worktree::WorktreeManager;

pub struct HybridMode;

impl super::ModeHandler for HybridMode {
    fn bring_up(
        &self,
        slug: &str,
        slot: u8,
        offset: u16,
        branch: &str,
        config: &Config,
        root: &Path,
        watch: bool,
    ) -> Result<Session> {
        let wt = WorktreeManager::new(root.to_owned());
        let worktree_path = wt.worktree_path(config, slug);

        let compose_path = compose::find_compose_file(root)
            .ok_or_else(|| crate::error::EcluseError::ComposeFileNotFound(root.display().to_string()))?;

        let compose_data = compose::parse(&compose_path)?;

        // Partition services
        let app_svcs = compose::app_services(&compose_data, &config.app_label, &config.app_label_value);
        let data_svcs = compose::data_services(&compose_data, &config.app_label, &config.app_label_value);

        if app_svcs.is_empty() {
            tracing::warn!(
                "No service labeled {}={} found; treating all services as data. \
                Add the label to your app service for proper hybrid mode behavior.",
                config.app_label,
                config.app_label_value
            );
        }

        let suffix = format!("{}_{}", config.prefix, slug);
        let overlay_dir = root.join(".ecluse").join("overlays");
        std::fs::create_dir_all(&overlay_dir)
            .context("failed to create overlays directory")?;
        let overlay_path = overlay_dir.join(format!("{}.yml", slug));

        // Generate overlay for data services only
        let overlay_yaml = compose::generate_overlay(&compose_data, offset, &suffix, Some(&data_svcs))?;
        std::fs::write(&overlay_path, &overlay_yaml)
            .context("failed to write overlay file")?;

        // Create worktree
        wt.create(&worktree_path, branch)?;

        let project = format!("{}_{}", config.prefix, slug);
        let compose_str = compose_path.to_string_lossy().to_string();
        let overlay_str = overlay_path.to_string_lossy().to_string();

        // Bring up data services only
        let data_svc_refs: Vec<&str> = data_svcs.iter().map(|s| s.as_str()).collect();
        if let Err(e) = docker::compose_up_services(&project, &compose_str, Some(&overlay_str), &data_svc_refs, watch) {
            let _ = wt.remove(&worktree_path);
            let _ = std::fs::remove_file(&overlay_path);
            return Err(e);
        }

        // Provision database if configured
        let database_name = if config.is_db_enabled() {
            let pg = PgClient::from_config(&config.database);
            if !pg.is_reachable() {
                let _ = docker::compose_down(&project, &compose_str, Some(&overlay_str), true);
                let _ = wt.remove(&worktree_path);
                let _ = std::fs::remove_file(&overlay_path);
                return Err(crate::error::EcluseError::PostgresUnreachable.into());
            }
            let db_name = PgClient::db_name(&config.database.base, slug);
            if let Err(e) = pg.create_db(&db_name) {
                let _ = docker::compose_down(&project, &compose_str, Some(&overlay_str), true);
                let _ = wt.remove(&worktree_path);
                let _ = std::fs::remove_file(&overlay_path);
                return Err(e);
            }
            Some(db_name)
        } else {
            None
        };

        let app_port = Some(offset); // first port in range for host-side app

        // Build data service port mapping
        let data_service_ports: Vec<(String, u16)> = compose_data
            .services
            .iter()
            .filter_map(|(name, svc)| {
                if data_svcs.contains(name) {
                    compose::service_host_port(svc, offset).map(|p| (name.clone(), p))
                } else {
                    None
                }
            })
            .collect();

        let env_map = env::build_env(
            slot,
            offset,
            "hybrid",
            app_port,
            database_name.as_deref(),
            &data_service_ports,
        );
        env::write_env_file(&worktree_path, &env_map)?;

        Ok(Session {
            slug: slug.to_string(),
            mode: crate::config::Mode::Hybrid,
            slot,
            offset,
            branch: branch.to_string(),
            worktree_path: worktree_path.display().to_string(),
            compose_project: Some(project),
            overlay_file: Some(overlay_str),
            app_port,
            database_name,
            started_at: Utc::now().to_rfc3339(),
        })
    }

    fn bring_down(
        &self,
        session: &Session,
        config: &Config,
        root: &Path,
        keep_volumes: bool,
        keep_database: bool,
    ) -> Result<()> {
        // Drop database
        if !keep_database {
            if let Some(db) = &session.database_name {
                let pg = PgClient::from_config(&config.database);
                pg.drop_db(db)?;
            }
        }

        // Bring down data containers
        if let Some(project) = &session.compose_project {
            if let Some(overlay) = &session.overlay_file {
                if let Some(compose_path) = compose::find_compose_file(root) {
                    let compose_str = compose_path.to_string_lossy().to_string();
                    docker::compose_down(project, &compose_str, Some(overlay), !keep_volumes)?;
                }
            }
            if let Some(overlay) = &session.overlay_file {
                let _ = std::fs::remove_file(overlay);
            }
        }

        // Remove worktree
        let wt = WorktreeManager::new(root.to_owned());
        let wt_path = std::path::PathBuf::from(&session.worktree_path);
        wt.remove(&wt_path)?;

        Ok(())
    }
}
