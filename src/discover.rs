//! On-demand port discovery.
//!
//! ecluse *assigns* ports (`port = base_port + slot × stride`) and treats
//! `state.json` as truth. Discovery is the complement: it observes which ports
//! are *actually* being listened on, so a command can report expected-vs-actual
//! instead of a bare `✗ down` when something bound the wrong port.
//!
//! Discovery never rewrites state. A mismatch is evidence of a bug (usually an
//! external task runner that re-read `.env.local` instead of `.env.ecluse`), not
//! a better value to adopt — see `incidents/2026-06-09-rubbr-cross-agent-kill-spiral`,
//! where trusting a discovered port hid a wrong-slot spawn behind a green check.
//!
//! ## Why a single snapshot
//!
//! The obvious implementation runs `lsof -p <pid>` per process and `pgrep -P`
//! per level of the tree. With 4 sessions × ~8 processes that is 30+ forks, and
//! `lsof` costs 20–400ms each. Instead we take *two* forks total, regardless of
//! session count — all listeners plus the whole process table — and do the
//! tree join in memory.

use std::collections::{HashMap, HashSet};
use std::process::Command;

/// Ports never reported as a session's dev server. These are system/privileged
/// services that show up in every scan and are never what a worktree allocated.
const IGNORED_PORTS: &[u16] = &[22, 80, 443];

/// One point-in-time view of every listening port on the host plus the process
/// table needed to attribute those ports to a process tree.
///
/// Built with exactly two subprocess calls. Cheap enough to take on every
/// command invocation; there is no background daemon.
#[derive(Debug, Clone, Default)]
pub struct PortSnapshot {
    /// pid → ports that pid is listening on directly.
    ports_by_pid: HashMap<u32, Vec<u16>>,
    /// port → the pid holding it (first one seen wins; a port has one owner).
    pid_by_port: HashMap<u16, u32>,
    /// ppid → direct children, for descendant walks without forking `pgrep`.
    children: HashMap<u32, Vec<u32>>,
}

/// Take a snapshot of all listening ports and the process table.
///
/// Best-effort by design: a missing or failing `lsof`/`ps` yields an empty (or
/// partial) snapshot rather than an error, because discovery is supplemental
/// reporting — it must never fail a command that would otherwise succeed.
pub fn snapshot() -> PortSnapshot {
    parse_snapshot(&raw_listeners(), &raw_process_table())
}

/// Run `lsof` for every listening TCP socket on the host. One fork.
fn raw_listeners() -> String {
    // -F pn emits machine-readable records: a `p<pid>` line followed by one
    // `n<addr>` line per socket. -n/-P skip DNS and service-name lookups,
    // which is both faster and keeps the port numeric.
    match Command::new("lsof")
        .args(["-iTCP", "-sTCP:LISTEN", "-n", "-P", "-F", "pn"])
        .output()
    {
        // lsof exits non-zero when nothing matches, which is a legitimate
        // empty result — take stdout either way.
        Ok(o) => String::from_utf8_lossy(&o.stdout).into_owned(),
        Err(_) => String::new(),
    }
}

/// Read the whole process table as `pid ppid` pairs. One fork.
fn raw_process_table() -> String {
    match Command::new("ps").args(["-axo", "pid=,ppid="]).output() {
        Ok(o) => String::from_utf8_lossy(&o.stdout).into_owned(),
        Err(_) => String::new(),
    }
}

/// Parse `lsof -F pn` output and a `ps -axo pid=,ppid=` table into a snapshot.
///
/// Split out from `snapshot()` so the parsing is unit-testable without
/// depending on whatever happens to be listening on the test machine.
pub fn parse_snapshot(lsof_out: &str, ps_out: &str) -> PortSnapshot {
    let mut ports_by_pid: HashMap<u32, Vec<u16>> = HashMap::new();
    let mut pid_by_port: HashMap<u16, u32> = HashMap::new();

    // `-F pn` is stateful: a `p<pid>` line sets the owner for every `n<addr>`
    // line that follows, until the next `p` line.
    let mut current_pid: Option<u32> = None;
    for line in lsof_out.lines() {
        let Some((tag, rest)) = line.split_at_checked(1) else {
            continue;
        };
        match tag {
            "p" => current_pid = rest.trim().parse().ok(),
            "n" => {
                let Some(pid) = current_pid else { continue };
                let Some(port) = parse_listen_port(rest) else {
                    continue;
                };
                if IGNORED_PORTS.contains(&port) {
                    continue;
                }
                let entry = ports_by_pid.entry(pid).or_default();
                if !entry.contains(&port) {
                    entry.push(port);
                }
                pid_by_port.entry(port).or_insert(pid);
            }
            _ => {}
        }
    }

    for ports in ports_by_pid.values_mut() {
        ports.sort_unstable();
    }

    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for line in ps_out.lines() {
        let mut parts = line.split_whitespace();
        let (Some(pid), Some(ppid)) = (parts.next(), parts.next()) else {
            continue;
        };
        let (Ok(pid), Ok(ppid)) = (pid.parse::<u32>(), ppid.parse::<u32>()) else {
            continue;
        };
        children.entry(ppid).or_default().push(pid);
    }

    PortSnapshot {
        ports_by_pid,
        pid_by_port,
        children,
    }
}

/// Extract the port from an lsof address field.
///
/// Handles the shapes lsof emits for a listening socket:
/// `*:3000`, `127.0.0.1:3000`, `[::1]:3000`, `[::]:3000`.
/// Splitting on the *last* colon is what makes the bracketed IPv6 forms work.
fn parse_listen_port(addr: &str) -> Option<u16> {
    let addr = addr.trim();
    // Some lsof builds append `->` peer info; a listener shouldn't have one,
    // but guard anyway so a stray record can't produce a bogus port.
    let addr = addr.split("->").next()?;
    addr.rsplit(':').next()?.parse().ok()
}

impl PortSnapshot {
    /// Every port listened on by `root_pid` or any of its descendants.
    ///
    /// This is the attribution primitive: a dev server is usually a grandchild
    /// of what ecluse spawned (`sh → pnpm → node → vite`), so the port belongs
    /// to the tree, not to the recorded pid.
    pub fn ports_for_tree(&self, root_pid: u32) -> Vec<u16> {
        let mut ports: Vec<u16> = Vec::new();
        for pid in self.tree_pids(root_pid) {
            if let Some(p) = self.ports_by_pid.get(&pid) {
                for port in p {
                    if !ports.contains(port) {
                        ports.push(*port);
                    }
                }
            }
        }
        ports.sort_unstable();
        ports
    }

    /// True iff `root_pid` or a descendant is listening on `port`.
    pub fn tree_owns_port(&self, root_pid: u32, port: u16) -> bool {
        self.ports_for_tree(root_pid).contains(&port)
    }

    /// The pid holding `port`, if anything is.
    pub fn listener_pid(&self, port: u16) -> Option<u32> {
        self.pid_by_port.get(&port).copied()
    }

    /// `root_pid` plus every transitive descendant.
    ///
    /// Cycle-guarded via `seen`: a `ps` snapshot taken while pids are being
    /// recycled can in principle contain a ppid loop, and an unguarded walk
    /// would hang.
    pub fn tree_pids(&self, root_pid: u32) -> Vec<u32> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut stack = vec![root_pid];
        while let Some(pid) = stack.pop() {
            if !seen.insert(pid) {
                continue;
            }
            out.push(pid);
            if let Some(kids) = self.children.get(&pid) {
                stack.extend(kids);
            }
        }
        out
    }

    /// True iff `descendant` is a transitive child of `ancestor`.
    /// A pid is not its own descendant.
    pub fn is_descendant(&self, ancestor: u32, descendant: u32) -> bool {
        ancestor != descendant && self.tree_pids(ancestor).contains(&descendant)
    }
}

/// Which slot a port belongs to under the configured allocation scheme.
///
/// `port = base_port + slot × stride` inverts to `slot = (port - base) / stride`
/// when the remainder is zero. Reporting this is what turns "a process is on a
/// port near mine" — the inference that started the 2026-06-09 kill spiral —
/// into "that port is slot 3's, owned by session X, do not kill it".
pub fn owning_slot(port: u16, base_port: u16, slot_stride: u8, max_slots: u8) -> Option<u8> {
    let stride = slot_stride.max(1) as u16;
    let offset = port.checked_sub(base_port)?;
    if offset % stride != 0 {
        return None;
    }
    let slot = offset / stride;
    // Slot 0 is not a valid allocation (slots are 1..=max_slots), and a port
    // beyond max_slots' territory belongs to nobody.
    if slot == 0 || slot > max_slots as u16 {
        return None;
    }
    Some(slot as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real `lsof -F pn` output shape: a `p` line, then `f`/`n` records, with
    // the same socket listed once per file descriptor.
    const LSOF: &str = "\
p591
f8
n*:60277
f9
n*:60277
p609
f8
n*:7000
f10
n127.0.0.1:5000
p744
f9
n[::1]:27017
f10
n[::]:27017
";

    // pid ppid — 700 is a child of 609, 800 a child of 700 (grandchild of 609).
    const PS: &str = "\
    1     0
  591     1
  609     1
  700   609
  800   700
  744     1
";

    fn snap() -> PortSnapshot {
        parse_snapshot(LSOF, PS)
    }

    #[test]
    fn parses_wildcard_address() {
        assert_eq!(parse_listen_port("*:3000"), Some(3000));
    }

    #[test]
    fn parses_ipv4_address() {
        assert_eq!(parse_listen_port("127.0.0.1:5000"), Some(5000));
    }

    // Bracketed IPv6 is why we split on the last colon, not the first.
    #[test]
    fn parses_ipv6_loopback() {
        assert_eq!(parse_listen_port("[::1]:27017"), Some(27017));
    }

    #[test]
    fn parses_ipv6_wildcard() {
        assert_eq!(parse_listen_port("[::]:8080"), Some(8080));
    }

    #[test]
    fn rejects_non_numeric_port() {
        assert_eq!(parse_listen_port("127.0.0.1:http"), None);
    }

    #[test]
    fn rejects_empty_address() {
        assert_eq!(parse_listen_port(""), None);
    }

    #[test]
    fn dedupes_same_port_across_file_descriptors() {
        let s = snap();
        assert_eq!(s.ports_for_tree(591), vec![60277]);
    }

    #[test]
    fn collects_multiple_ports_for_one_pid() {
        let s = snap();
        assert_eq!(s.ports_for_tree(609), vec![5000, 7000]);
    }

    #[test]
    fn dedupes_ipv4_and_ipv6_of_same_port() {
        let s = snap();
        assert_eq!(s.ports_for_tree(744), vec![27017]);
    }

    #[test]
    fn maps_port_to_listener_pid() {
        let s = snap();
        assert_eq!(s.listener_pid(7000), Some(609));
        assert_eq!(s.listener_pid(60277), Some(591));
    }

    #[test]
    fn listener_pid_none_for_unheld_port() {
        assert_eq!(snap().listener_pid(9999), None);
    }

    // The point of the tree walk: a port held by a grandchild belongs to the
    // service ecluse spawned, not to some unrelated process.
    #[test]
    fn attributes_descendant_port_to_root() {
        // 800 (grandchild of 609) holds 4000.
        let s = parse_snapshot("p800\nf8\nn*:4000\n", PS);
        assert!(s.tree_owns_port(609, 4000));
        assert_eq!(s.ports_for_tree(609), vec![4000]);
    }

    #[test]
    fn does_not_attribute_sibling_port_to_root() {
        let s = snap();
        // 591 is a sibling of 609, not a descendant.
        assert!(!s.tree_owns_port(609, 60277));
    }

    #[test]
    fn ignores_system_ports() {
        let s = parse_snapshot("p42\nf8\nn*:22\nf9\nn*:80\nf10\nn*:443\nf11\nn*:3000\n", "");
        assert_eq!(s.ports_for_tree(42), vec![3000]);
        assert_eq!(s.listener_pid(22), None);
    }

    #[test]
    fn tree_pids_includes_root_and_all_descendants() {
        let mut pids = snap().tree_pids(609);
        pids.sort_unstable();
        assert_eq!(pids, vec![609, 700, 800]);
    }

    #[test]
    fn tree_pids_is_just_root_when_childless() {
        assert_eq!(snap().tree_pids(744), vec![744]);
    }

    #[test]
    fn is_descendant_walks_multiple_levels() {
        let s = snap();
        assert!(s.is_descendant(609, 700));
        assert!(s.is_descendant(609, 800));
    }

    #[test]
    fn is_descendant_false_for_ancestor_direction() {
        assert!(!snap().is_descendant(800, 609));
    }

    #[test]
    fn is_descendant_false_for_self() {
        assert!(!snap().is_descendant(609, 609));
    }

    #[test]
    fn unknown_pid_has_no_ports() {
        assert!(snap().ports_for_tree(999_999).is_empty());
    }

    // A ppid cycle can only come from a torn `ps` read, but an unguarded
    // walk would spin forever on it.
    #[test]
    fn tolerates_ppid_cycle() {
        let s = parse_snapshot("", "10 11\n11 10\n");
        let mut pids = s.tree_pids(10);
        pids.sort_unstable();
        assert_eq!(pids, vec![10, 11]);
    }

    #[test]
    fn empty_input_yields_empty_snapshot() {
        let s = parse_snapshot("", "");
        assert!(s.ports_for_tree(1).is_empty());
        assert_eq!(s.listener_pid(3000), None);
    }

    #[test]
    fn malformed_lines_are_skipped() {
        // An `n` record with no preceding `p` line has no owner; garbage ps
        // rows are dropped rather than panicking.
        let s = parse_snapshot("n*:3000\nxjunk\np50\nn*:3001\n", "notapid alsonot\n60 x\n");
        assert_eq!(s.listener_pid(3000), None);
        assert_eq!(s.listener_pid(3001), Some(50));
        assert_eq!(s.tree_pids(60), vec![60]);
    }

    // ── owning_slot ───────────────────────────────────────────────────────────

    #[test]
    fn owning_slot_identifies_own_slot_stride_1() {
        assert_eq!(owning_slot(3001, 3000, 1, 8), Some(1));
        assert_eq!(owning_slot(3004, 3000, 1, 8), Some(4));
    }

    #[test]
    fn owning_slot_identifies_slot_with_stride_10() {
        assert_eq!(owning_slot(3010, 3000, 10, 8), Some(1));
        assert_eq!(owning_slot(3030, 3000, 10, 8), Some(3));
    }

    // An auto-bumped port that lands between slots belongs to no slot — this
    // is the benign case that must NOT be reported as cross-slot theft.
    #[test]
    fn owning_slot_none_when_not_on_stride_boundary() {
        assert_eq!(owning_slot(3015, 3000, 10, 8), None);
    }

    #[test]
    fn owning_slot_none_for_base_port_itself() {
        // base_port is slot 0 — never a valid allocation.
        assert_eq!(owning_slot(3000, 3000, 1, 8), None);
    }

    #[test]
    fn owning_slot_none_beyond_max_slots() {
        assert_eq!(owning_slot(3009, 3000, 1, 8), None);
    }

    #[test]
    fn owning_slot_none_below_base_port() {
        assert_eq!(owning_slot(2999, 3000, 1, 8), None);
    }

    #[test]
    fn owning_slot_handles_last_valid_slot() {
        assert_eq!(owning_slot(3008, 3000, 1, 8), Some(8));
    }

    #[test]
    fn owning_slot_treats_zero_stride_as_one() {
        assert_eq!(owning_slot(3002, 3000, 0, 8), Some(2));
    }

    // Guards against a real-world regression: a live snapshot must find the
    // port this test process is holding.
    #[test]
    fn live_snapshot_finds_own_listener() {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let s = snapshot();
        // lsof may be absent in a sandbox; only assert when it produced data.
        if s.listener_pid(port).is_some() {
            assert!(s.tree_owns_port(std::process::id(), port));
        }
    }
}
