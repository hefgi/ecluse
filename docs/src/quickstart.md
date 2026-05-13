# Quick start

```bash
cd my-project
ecluse init              # detects mode, writes .ecluse.toml
ecluse up feat-foo       # creates worktree + slot
ecluse shell feat-foo    # drops into worktree with env loaded
npm run dev              # PORT, DATABASE_URL, etc. already set
```

Your app runs on a unique port. Other sessions run in parallel without touching yours. Type `exit` to leave the session.

## Parallel sessions

```bash
ecluse up feat-foo    # slot 1 — PORT=3001
ecluse up fix-bar     # slot 2 — PORT=3002
ecluse up chore-baz   # slot 3 — PORT=3003
```

Each session is fully independent. Tear down in any order:

```bash
ecluse down feat-foo
ecluse ls               # fix-bar and chore-baz still running
```

## Soft restart

Tear down services without losing your worktree, then spin them up fresh:

```bash
ecluse down feat-foo --keep-worktree   # services torn down, worktree + branch kept
ecluse up feat-foo --reuse-worktree    # new slot, fresh ports, worktree reused
```
