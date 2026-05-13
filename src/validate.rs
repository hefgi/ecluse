use anyhow::Result;
use std::process::Command;

use crate::config::{Config, ServiceConfig};

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
    let candidates = std::iter::once(nominal).chain(
        (1..=config.port_search_range as u16).map(|i| nominal + i * max_slots),
    );

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

/// Validate that `port_search_range` is safe given the service layout.
///
/// For every pair of adjacent services (sorted by base_port), the highest port
/// that slot 1 of the lower service could reach must not reach the lowest port
/// that slot 1 of the upper service could have.
///
/// Upper bound of lower service's search space:
///   lower.base_port + max_slots + port_search_range * max_slots
///   = lower.base_port + max_slots * (port_search_range + 1)
///
/// Must be < upper.base_port + 1  (i.e., strictly below slot 1 of upper)
pub fn validate_config(config: &Config) -> Result<Vec<String>> {
    let mut warnings: Vec<String> = vec![];
    let mut errors: Vec<String> = vec![];

    if config.services.is_empty() {
        return Ok(warnings);
    }

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

    fn make_config(
        max_slots: u8,
        port_search_range: u8,
        services: Vec<ServiceConfig>,
    ) -> Config {
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
        }
    }

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
        let config = make_config(
            8,
            10,
            vec![svc("a", 3000), svc("b", 3100), svc("c", 3150)],
        );
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
}
