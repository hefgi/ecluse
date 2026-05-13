pub struct Skill {
    pub name: &'static str,
    pub description: &'static str,
    pub content: &'static str,
}

pub const ALL_SKILLS: &[Skill] = &[
    Skill {
        name: "getting-started",
        description: "Install, init, the three commands. Five-minute on-ramp.",
        content: include_str!("skills/getting-started.md"),
    },
    Skill {
        name: "choosing-a-mode",
        description: "Decision guide: container, host, or hybrid. Signal table.",
        content: include_str!("skills/choosing-a-mode.md"),
    },
    Skill {
        name: "agent-workflow",
        description: "Canonical loop for coding agents using ecluse.",
        content: include_str!("skills/agent-workflow.md"),
    },
    Skill {
        name: "container-mode",
        description: "Full-stack containerized sessions. Port offsets and volume namespacing.",
        content: include_str!("skills/container-mode.md"),
    },
    Skill {
        name: "host-mode",
        description: "Native dev stack with database provisioning. No Docker required.",
        content: include_str!("skills/host-mode.md"),
    },
    Skill {
        name: "hybrid-mode",
        description: "Data in containers, app on host. The ecluse.role label.",
        content: include_str!("skills/hybrid-mode.md"),
    },
    Skill {
        name: "troubleshooting",
        description: "Port in use, Docker down, stale state, Postgres unreachable, and more.",
        content: include_str!("skills/troubleshooting.md"),
    },
    Skill {
        name: "limits",
        description: "What ecluse intentionally does not do in v0.",
        content: include_str!("skills/limits.md"),
    },
];

pub fn find(name: &str) -> Option<&'static Skill> {
    ALL_SKILLS.iter().find(|s| s.name == name)
}
