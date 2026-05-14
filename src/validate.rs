use anyhow::Result;
use std::collections::HashSet;
use std::process::Command;

use crate::config::{Config, Mode, ServiceConfig, ServiceRun};
use crate::process::ProcessManager;

/// Check if a TCP port is currently listening on localhost.
/// Returns the PID of the process holding the port, or 0 if unknown.
fn port_listener_pid(port: u16) -> Option<u32> {
    let output = Command::new("lsof")
        .args(["-iTCP", &format!(":{}", port), "-sTCP:LISTEN", "-n", "-P"])
        .output()
        .ok()?;

    if output.status.success() && !output.stdout.is_empty() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let pid: u32 = stdout
            .lines()
            .nth(1)
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|p| p.parse().ok())
            .unwrap_or(0);
        Some(pid)
    } else {
        None
    }
}

/// Find a free port for a service on a given slot.
///
/// Tries: `nominal`, `nominal + max_slots`, `nominal + 2*max_slots`, …
/// up to `port_search_range` additional candidates (so up to range+1 total attempts).
///
/// Skipping by `max_slots` ensures we never land on another slot's nominal port
/// for the same service.
///
/// In strict mode, fails immediately if the nominal port is in use.
pub fn find_free_port(config: &Config, service: &ServiceConfig, slot: u8) -> Result<u16> {
    let nominal = service.port(slot);
    let max_slots = config.max_slots as u16;

    if config.strict_port {
        if let Some(pid) = port_listener_pid(nominal) {
            return Err(crate::error::EcluseError::PortInUse { port: nominal, pid }.into());
        }
        return Ok(nominal);
    }

    // Try nominal port first, then bump by max_slots increments
    let candidates = std::iter::once(nominal)
        .chain((1..=config.port_search_range as u16).map(|i| nominal + i * max_slots));

    for port in candidates {
        if port_listener_pid(port).is_none() {
            if port != nominal {
                tracing::info!(
                    "port {} in use; using {} for service '{}'",
                    nominal,
                    port,
                    service.name
                );
            }
            return Ok(port);
        }
    }

    Err(crate::error::EcluseError::PortExhausted {
        service: service.name.clone(),
        nominal,
        range: config.port_search_range,
    }
    .into())
}

/// Validate that the configured process manager binary is available.
pub fn validate_process_manager(manager: &ProcessManager) -> Result<()> {
    match manager {
        ProcessManager::None => Ok(()),
        ProcessManager::Tmux => {
            if crate::process::binary_available("tmux") {
                Ok(())
            } else {
                Err(crate::error::EcluseError::ProcessManagerUnavailable {
                    name: "tmux".into(),
                }
                .into())
            }
        }
        ProcessManager::Nohup => {
            if crate::process::binary_available("nohup") {
                Ok(())
            } else {
                Err(crate::error::EcluseError::ProcessManagerUnavailable {
                    name: "nohup".into(),
                }
                .into())
            }
        }
    }
}

/// Validate that the config is structurally sound and that `port_search_range`
/// is safe given the service layout.
pub fn validate_config(config: &Config) -> Result<Vec<String>> {
    let mut warnings: Vec<String> = vec![];
    let mut errors: Vec<String> = vec![];

    let name_re = regex_lite::Regex::new(r"^[a-z0-9][a-z0-9_-]*$").unwrap();

    // prefix
    if config.prefix.is_empty() {
        errors.push("prefix must not be empty".into());
    } else if !name_re.is_match(&config.prefix) {
        errors.push(format!(
            "prefix '{}' contains invalid characters; only [a-z0-9_-] allowed",
            config.prefix
        ));
    }

    // max_slots
    if config.max_slots == 0 {
        errors.push("max_slots must be at least 1".into());
    } else if config.max_slots > 64 {
        warnings.push(format!(
            "max_slots is {} which is unusually high; verify this is intentional",
            config.max_slots
        ));
    }

    // per-service checks
    for (i, svc) in config.services.iter().enumerate() {
        if svc.name.is_empty() {
            errors.push(format!("service at index {} has an empty name", i));
        } else if !name_re.is_match(&svc.name) {
            errors.push(format!(
                "service name '{}' contains invalid characters; only [a-z0-9_-] allowed",
                svc.name
            ));
        }

        if svc.base_port == 0 {
            errors.push(format!("service '{}': base_port must not be 0", svc.name));
        } else if svc.base_port < 1024 {
            warnings.push(format!(
                "service '{}': base_port {} is a privileged port (< 1024); binding may require elevated privileges",
                svc.name, svc.base_port
            ));
        }

        // No compose field is fine — the service will use the root compose file.
        // Only flag when the service has an explicit compose path that doesn't exist.
        if let Some(ref compose_rel) = svc.compose {
            // We can't check the path here without the repo root, so just validate format.
            if compose_rel.is_empty() {
                errors.push(format!(
                    "service '{}': compose path must not be empty",
                    svc.name
                ));
            }
        }
        if svc.run == ServiceRun::Native && svc.compose.is_some() {
            warnings.push(format!(
                "service '{}': compose is set but run is 'native'; the compose field will be ignored",
                svc.name
            ));
        }

        if config.mode == Mode::Host && svc.run == ServiceRun::Docker {
            errors.push(format!(
                "service '{}': run is 'docker' but mode is 'host'; host mode does not run containers",
                svc.name
            ));
        }
    }

    // duplicate service names
    let mut seen_names: HashSet<&str> = HashSet::new();
    for svc in &config.services {
        if !seen_names.insert(svc.name.as_str()) {
            errors.push(format!("duplicate service name '{}'", svc.name));
        }
    }

    // duplicate base_ports (skip port 0, already reported above)
    let mut seen_ports: HashSet<u16> = HashSet::new();
    for svc in &config.services {
        if svc.base_port != 0 && !seen_ports.insert(svc.base_port) {
            errors.push(format!(
                "duplicate base_port {}: multiple services share the same base port",
                svc.base_port
            ));
        }
    }

    // adjacent gap check
    if !config.services.is_empty() {
        let mut sorted: Vec<&ServiceConfig> = config.services.iter().collect();
        sorted.sort_by_key(|s| s.base_port);

        let max_slots = config.max_slots as u32;
        let range = config.port_search_range as u32;

        for window in sorted.windows(2) {
            let lower = window[0];
            let upper = window[1];

            // Highest port the lower service could ever use (any slot, any bump)
            let lower_max = lower.base_port as u32 + max_slots + range * max_slots;
            // Lowest port the upper service uses (slot 1 nominal)
            let upper_min = upper.base_port as u32 + 1;

            if lower_max >= upper_min {
                errors.push(format!(
                    "service '{}' (base_port={}) search space reaches port {} which overlaps service '{}' (base_port={}); \
                     increase the gap between base_port values or reduce port_search_range (currently {})",
                    lower.name, lower.base_port, lower_max,
                    upper.name, upper.base_port,
                    config.port_search_range,
                ));
            }
        }
    }

    // Warn if port_search_range is 0 (effectively strict without being explicit)
    if config.port_search_range == 0 && !config.strict_port {
        warnings.push(
            "port_search_range is 0 with strict_port=false; set strict_port=true to be explicit"
                .into(),
        );
    }

    if !errors.is_empty() {
        return Err(crate::error::EcluseError::ConfigInvalid(errors.join("; ")).into());
    }

    Ok(warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{HookConfig, Mode, ServiceRun};

    fn make_config(max_slots: u8, port_search_range: u8, services: Vec<ServiceConfig>) -> Config {
        Config {
            mode: Mode::Hybrid,
            max_slots,
            prefix: "ecluse".into(),
            worktree_dir: ".ecluse/worktrees".into(),
            app_label: "ecluse.role".into(),
            app_label_value: "app".into(),
            strict_port: false,
            port_search_range,
            services,
            hooks: HookConfig::default(),
        }
    }

    fn svc(name: &str, base_port: u16) -> ServiceConfig {
        ServiceConfig {
            name: name.into(),
            base_port,
            run: ServiceRun::Native,
            compose: None,
            command: None,
            port_env: vec![],
        }
    }

    fn svc_docker(name: &str, base_port: u16, compose: Option<&str>) -> ServiceConfig {
        ServiceConfig {
            name: name.into(),
            base_port,
            run: ServiceRun::Docker,
            compose: compose.map(|s| s.into()),
            command: None,
            port_env: vec![],
        }
    }

    // --- existing tests (unchanged) ---

    #[test]
    fn no_services_is_valid() {
        let config = make_config(8, 10, vec![]);
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn single_service_is_valid() {
        let config = make_config(8, 10, vec![svc("api", 3000)]);
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn services_with_sufficient_gap_are_valid() {
        // lower: base=3000, max_slots=4, range=3 → max = 3000 + 4 + 3*4 = 3016
        // upper: base=3100 → min = 3101
        // 3016 < 3101 ✓
        let config = make_config(4, 3, vec![svc("api", 3000), svc("web", 3100)]);
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn services_with_insufficient_gap_are_invalid() {
        // lower: base=3000, max_slots=8, range=10 → max = 3000 + 8 + 10*8 = 3088
        // upper: base=3050 → min = 3051
        // 3088 >= 3051 ✗
        let config = make_config(8, 10, vec![svc("api", 3000), svc("web", 3050)]);
        let err = validate_config(&config).unwrap_err();
        assert!(err.to_string().contains("overlaps"));
    }

    #[test]
    fn validation_checks_all_adjacent_pairs() {
        // a→b ok, b→c not ok
        // b: base=3100, max_slots=8, range=10 → max = 3100+8+80 = 3188
        // c: base=3150 → min = 3151
        // 3188 >= 3151 ✗
        let config = make_config(8, 10, vec![svc("a", 3000), svc("b", 3100), svc("c", 3150)]);
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn validation_sorts_by_base_port_not_definition_order() {
        // same as sufficient_gap test but services defined in reverse order
        let config = make_config(4, 3, vec![svc("web", 3100), svc("api", 3000)]);
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn exact_boundary_is_invalid() {
        // lower: base=3000, max_slots=4, range=3 → max = 3000+4+12 = 3016
        // upper: base=3016 → min = 3017
        // 3016 < 3017 ✓ — should be valid
        let config = make_config(4, 3, vec![svc("api", 3000), svc("web", 3016)]);
        // upper_min = 3017, lower_max = 3016, so 3016 < 3017 → valid
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn one_below_boundary_is_invalid() {
        // lower: base=3000, max_slots=4, range=3 → max = 3016
        // upper: base=3015 → min = 3016
        // 3016 >= 3016 ✗
        let config = make_config(4, 3, vec![svc("api", 3000), svc("web", 3015)]);
        assert!(validate_config(&config).is_err());
    }

    // --- prefix ---

    #[test]
    fn prefix_empty_is_error() {
        let mut config = make_config(8, 10, vec![]);
        config.prefix = "".into();
        let err = validate_config(&config).unwrap_err();
        assert!(err.to_string().contains("prefix"));
    }

    #[test]
    fn prefix_with_space_is_error() {
        let mut config = make_config(8, 10, vec![]);
        config.prefix = "my app".into();
        let err = validate_config(&config).unwrap_err();
        assert!(err.to_string().contains("my app"));
    }

    #[test]
    fn prefix_with_uppercase_is_error() {
        let mut config = make_config(8, 10, vec![]);
        config.prefix = "MyApp".into();
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn prefix_with_hyphen_is_ok() {
        let mut config = make_config(8, 10, vec![]);
        config.prefix = "my-app".into();
        assert!(validate_config(&config).is_ok());
    }

    // --- max_slots ---

    #[test]
    fn max_slots_zero_is_error() {
        let config = make_config(0, 10, vec![]);
        let err = validate_config(&config).unwrap_err();
        assert!(err.to_string().contains("max_slots"));
    }

    #[test]
    fn max_slots_65_warns() {
        let config = make_config(65, 0, vec![]);
        let warnings = validate_config(&config).unwrap();
        assert!(warnings.iter().any(|w| w.contains("unusually high")));
    }

    #[test]
    fn max_slots_64_no_high_warning() {
        let mut config = make_config(64, 10, vec![]);
        config.strict_port = true;
        let warnings = validate_config(&config).unwrap();
        assert!(!warnings.iter().any(|w| w.contains("unusually high")));
    }

    // --- service names ---

    #[test]
    fn service_name_empty_is_error() {
        let config = make_config(8, 10, vec![svc("", 3000)]);
        let err = validate_config(&config).unwrap_err();
        assert!(err.to_string().contains("empty name"));
    }

    #[test]
    fn service_name_with_space_is_error() {
        let config = make_config(8, 10, vec![svc("my service", 3000)]);
        let err = validate_config(&config).unwrap_err();
        assert!(err.to_string().contains("my service"));
    }

    #[test]
    fn service_name_with_uppercase_is_error() {
        let config = make_config(8, 10, vec![svc("Api", 3000)]);
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn service_name_valid_with_underscore_is_ok() {
        let config = make_config(8, 10, vec![svc("api_v2", 3000)]);
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn service_name_valid_with_hyphen_is_ok() {
        let config = make_config(8, 10, vec![svc("my-api", 3000)]);
        assert!(validate_config(&config).is_ok());
    }

    // --- base_port ---

    #[test]
    fn base_port_zero_is_error() {
        let config = make_config(8, 10, vec![svc("api", 0)]);
        let err = validate_config(&config).unwrap_err();
        assert!(err.to_string().contains("base_port"));
    }

    #[test]
    fn base_port_1023_warns() {
        let mut config = make_config(8, 0, vec![svc("api", 1023)]);
        config.strict_port = true;
        let warnings = validate_config(&config).unwrap();
        assert!(warnings.iter().any(|w| w.contains("privileged")));
    }

    #[test]
    fn base_port_1024_no_privileged_warning() {
        let mut config = make_config(8, 0, vec![svc("api", 1024)]);
        config.strict_port = true;
        let warnings = validate_config(&config).unwrap();
        assert!(!warnings.iter().any(|w| w.contains("privileged")));
    }

    // --- compose consistency ---

    #[test]
    fn docker_run_without_compose_is_valid() {
        // No explicit compose path is fine — the service uses the root compose file.
        let config = make_config(8, 10, vec![svc_docker("db", 5432, None)]);
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn docker_run_with_compose_is_ok() {
        let config = make_config(
            8,
            0,
            vec![svc_docker("db", 5432, Some("docker-compose.yml"))],
        );
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn native_run_with_compose_warns() {
        let config = make_config(
            8,
            10,
            vec![ServiceConfig {
                name: "api".into(),
                base_port: 3000,
                run: ServiceRun::Native,
                compose: Some("docker-compose.yml".into()),
                command: None,
                port_env: vec![],
            }],
        );
        let warnings = validate_config(&config).unwrap();
        assert!(warnings.iter().any(|w| w.contains("ignored")));
    }

    // --- mode × run ---

    #[test]
    fn host_mode_with_docker_service_is_error() {
        let mut config = make_config(
            8,
            10,
            vec![svc_docker("db", 5432, Some("docker-compose.yml"))],
        );
        config.mode = Mode::Host;
        let err = validate_config(&config).unwrap_err();
        assert!(err.to_string().contains("host"));
    }

    #[test]
    fn hybrid_mode_with_docker_service_is_ok() {
        let mut config = make_config(
            8,
            0,
            vec![svc_docker("db", 5432, Some("docker-compose.yml"))],
        );
        config.mode = Mode::Hybrid;
        config.strict_port = true;
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn container_mode_with_docker_service_is_ok() {
        let mut config = make_config(
            8,
            0,
            vec![svc_docker("db", 5432, Some("docker-compose.yml"))],
        );
        config.mode = Mode::Container;
        config.strict_port = true;
        assert!(validate_config(&config).is_ok());
    }

    // --- duplicates ---

    #[test]
    fn duplicate_service_names_is_error() {
        let config = make_config(8, 10, vec![svc("api", 3000), svc("api", 4000)]);
        let err = validate_config(&config).unwrap_err();
        assert!(err.to_string().contains("duplicate service name"));
        assert!(err.to_string().contains("api"));
    }

    #[test]
    fn duplicate_base_ports_is_error() {
        let config = make_config(8, 10, vec![svc("api", 3000), svc("web", 3000)]);
        let err = validate_config(&config).unwrap_err();
        assert!(err.to_string().contains("duplicate base_port"));
        assert!(err.to_string().contains("3000"));
    }

    // --- process manager ---

    #[test]
    fn validate_pm_none_always_ok() {
        assert!(validate_process_manager(&ProcessManager::None).is_ok());
    }

    #[test]
    fn validate_pm_nohup_when_present() {
        // nohup is always available on Unix
        assert!(validate_process_manager(&ProcessManager::Nohup).is_ok());
    }

    #[test]
    fn validate_pm_tmux_unavailable_returns_error() {
        // Only meaningful when tmux is not installed; skip otherwise
        if crate::process::binary_available("tmux") {
            return;
        }
        let err = validate_process_manager(&ProcessManager::Tmux).unwrap_err();
        assert!(err.to_string().contains("tmux"));
    }
}
