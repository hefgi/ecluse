# Demo recording scripts

Two assets, two jobs. Record them in this order.

1. **`demo.gif`** — 30s, silent, loops. Goes at the top of the README. Job: make a
   stranger understand the product in 10 seconds.
2. **`caught-bug.mp4`** — 90s, narrated. Goes in launch posts, the pitch deck, and
   the HN first comment. Job: prove agents catch real bugs when they have somewhere
   to verify.

The GIF sells the feature. The video sells the thesis. You need both, and the video
is the more valuable of the two.

---

## Setup (once, for both)

### Tooling

```bash
brew install asciinema agg ffmpeg   # asciinema → cast, agg → gif
# GUI alternative for the GIF: Kap (https://getkap.co) — brew install --cask kap
```

`asciinema` + `agg` gives crisp, small, text-perfect GIFs and is the right choice for
a terminal-only demo. Use Kap or QuickTime if you want to show a browser window.

### Terminal appearance

- **Font size 16–18pt.** GIFs get downscaled in the README; small text turns to mush.
- **Window 100×30 max.** Wider than that and the README render is unreadable.
- Dark theme, high contrast. Hide any personal information in the prompt.
- Simplify your prompt for the recording — a long path or git status eats horizontal space:
  ```bash
  PS1='$ '
  ```
- Clear scrollback right before you start: `clear && printf '\033[3J'`

### Demo repo

Do **not** record against a private or client repo. Build a small purpose-made repo
so the demo is reproducible and you can re-record when the CLI changes.

Requirements: Next.js or Express API + Postgres + Redis, one migration, a test suite
that runs in <10s, and one e2e test that hits a real endpoint. Commit a working
`.ecluse.toml` with `[[services]]` for api/postgres/redis and a `post_up` migration hook.

Suggested: `hefgi/ecluse-demo`, public, MIT. Link it from both assets so people can
reproduce.

---

## Asset 1 — `demo.gif` (30s, README hero)

**The single point to land:** four agents, four real stacks, one laptop, zero collisions.

### Shot list

| Time | Screen | Purpose |
|---|---|---|
| 0–3s | `ecluse up feat-auth` → slot 1, ports listed | Establish the primitive |
| 3–6s | `ecluse up feat-search` → slot 2, *different* ports | The isolation is visible |
| 6–9s | `ecluse up fix-cache` → slot 3 | Repetition = pattern |
| 9–12s | `ecluse up chore-deps` → slot 4 | Four is enough to make the point |
| 12–18s | `ecluse ls` — four sessions, four port sets, four DBs | **The money shot** |
| 18–24s | Split panes: 4 agents working simultaneously | Real parallelism |
| 24–28s | `curl localhost:3001/health` and `:3002/health` both 200 | Proof they're actually running |
| 28–30s | `ecluse shutdown` → clean | Teardown is part of the value |

### Recording

```bash
cd ~/dev/ecluse-demo
git checkout main && ecluse shutdown --delete-worktrees   # clean slate
clear && printf '\033[3J'

asciinema rec demo.cast --cols 100 --rows 30 --idle-time-limit 1.5
#   ... perform the shot list ...
#   exit

agg demo.cast demo.gif \
  --font-size 18 \
  --speed 1.4 \
  --theme monokai \
  --last-frame-duration 2

# Keep it under ~3 MB or GitHub throttles the render
ls -lh demo.gif
```

If it's over 3 MB: raise `--speed`, cut the split-pane section, or drop to 3 agents.

### Rules

- `--idle-time-limit 1.5` collapses your thinking pauses. Don't try to type fast.
- Never show a failure or a stray error in the GIF. Re-record instead.
- No typos. Viewers read every character in a loop.
- Test the render at 800px wide before committing — that's the README width.

### Install

```bash
cp demo.gif ~/dev/ecluse/docs/demo.gif
# then remove the TODO(demo) comment in README.md
```

---

## Asset 2 — `caught-bug.mp4` (90s, the thesis)

**The single point to land:** the agent found its own bug, because it had somewhere
to run the code. Same model, same prompt — the only difference is the environment.

This is the most valuable asset in the whole GTM plan. It's simultaneously the launch
video, the pitch-deck demo, and the answer to "why is this not just a worktree manager."

### The bug to plant

Pick a bug that **passes unit tests and fails against a real system**. That asymmetry
*is* the argument. Good candidates:

1. **N+1 query** — correct results, unit tests green with 3 seeded rows, obvious against a real DB with 10k rows.
2. **Missing migration** — code references a new column; unit tests mock the DB; the real Postgres 500s.
3. **Cache-key collision** — two endpoints share a Redis key. Passes in isolation, wrong data with a real Redis.
4. **Text overflow** — the Claude Code Desktop bug from Theo's video: renders fine in a
   unit test, visibly truncated in a browser. Best option if you can show a browser.

**Recommended: #2 (missing migration).** Fastest to stage, most legible in 90 seconds,
and impossible to catch by reading the diff — which is precisely the point.

### Structure

**Act 1 — the counterfactual (0:00–0:30)**
```
Agent, no environment. Task: "add a status column to orders and filter by it."
→ writes the code
→ runs unit tests: PASS (the DB is mocked)
→ opens a PR
Narration: "Tests pass. Code looks right. Nobody ran it. This is now in a human's queue."
```

**Act 2 — same task, with ecluse (0:30–1:10)**
```
ecluse up feat-order-status
→ post_up runs migrations against its own Postgres
→ agent writes the same code
→ unit tests: PASS
→ agent runs the e2e suite against its own live stack
→ FAIL: 500 on GET /orders?status=open — column "status" does not exist
→ agent reads the error, writes the missing migration, re-runs
→ PASS
Narration: "Same model. Same prompt. The only difference is that this one had
somewhere to check its work."
```

**Act 3 — the scale point (1:10–1:30)**
```
ecluse ls → 4 sessions, 4 stacks
Narration: "Four agents, four stacks, one laptop. Verification runs in parallel with
generation instead of queueing behind one human."
```

### Recording

```bash
# Rehearse until it runs clean end to end. Script the agent prompts
# in a file so they're identical across takes.

# Terminal-only:
asciinema rec caught-bug.cast --cols 110 --rows 32

# With a browser (recommended if using bug #4) — Kap or:
ffmpeg -f avfoundation -i "1" -r 30 -c:v libx264 -crf 18 caught-bug-raw.mp4
```

Add narration afterwards rather than talking live — you'll re-record less. A clean
subtitle track works too and travels better on X and LinkedIn, where most people watch
muted.

### Rules

- **Do not fake the failure.** The bug must genuinely fail e2e and genuinely pass units.
  Someone will try to reproduce it, and the demo repo makes that easy on purpose.
- Show the actual error text. `column "status" does not exist` is more persuasive than
  any claim you could write.
- Keep the agent's self-correction visible but trimmed — cut long thinking blocks.
- End on `ecluse ls`. The parallelism is the business case.

### Derivative cuts

From the same footage:
- **20s** — Act 2 only, for X and LinkedIn (muted-friendly, subtitles required)
- **10s GIF** — the FAIL → fix → PASS moment, for the blog posts and HN comment
- **Still frame** of the failing e2e output — the pitch-deck slide

---

## Verification checklist

Before either asset ships:

- [ ] Renders legibly at 800px wide (README) and on a phone
- [ ] Under 3 MB for the GIF
- [ ] No personal info: paths, hostnames, tokens, client names, real branch names
- [ ] No typos, no stray errors, no unexplained pauses
- [ ] Demo repo is public and linked
- [ ] The bug in the video genuinely reproduces from a clean clone
- [ ] Re-watched once at full speed without wincing
