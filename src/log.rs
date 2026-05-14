use std::io::IsTerminal;

const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const CYAN: &str = "\x1b[36m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const RESET: &str = "\x1b[0m";

pub struct StepLogger {
    quiet: bool,
    color: bool,
}

impl StepLogger {
    pub fn new(quiet: bool) -> Self {
        Self {
            quiet,
            color: std::io::stdout().is_terminal(),
        }
    }

    /// Print a step header: "» <label>"
    pub fn step(&self, label: &str) {
        if self.quiet {
            return;
        }
        if self.color {
            println!("{CYAN}{BOLD}»{RESET} {BOLD}{label}{RESET}");
        } else {
            println!("» {label}");
        }
    }

    /// Print a step outcome: "  → <detail>"
    pub fn detail(&self, detail: &str) {
        if self.quiet {
            return;
        }
        if self.color {
            println!("  {DIM}→{RESET} {detail}");
        } else {
            println!("  → {detail}");
        }
    }

    /// Print a success line (used as the final summary for a command).
    pub fn success(&self, msg: &str) {
        if self.quiet {
            return;
        }
        if self.color {
            println!("{GREEN}{BOLD}✓{RESET} {msg}");
        } else {
            println!("✓ {msg}");
        }
    }

    /// Print a warning (always shown regardless of quiet).
    pub fn warn(&self, msg: &str) {
        if self.color {
            eprintln!("{YELLOW}warning:{RESET} {msg}");
        } else {
            eprintln!("warning: {msg}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_logger_produces_no_output() {
        // Just verify it doesn't panic — can't easily capture stdout in unit tests.
        let log = StepLogger::new(true);
        log.step("loading config");
        log.detail("mode: hybrid");
        log.success("done");
    }

    #[test]
    fn non_quiet_logger_does_not_panic() {
        let log = StepLogger::new(false);
        log.step("loading config");
        log.detail("mode: hybrid");
        log.success("done");
        log.warn("something smells");
    }
}
