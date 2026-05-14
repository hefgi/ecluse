# Choosing a mode

`ecluse init` detects the right mode automatically. You confirm before anything is written.

| Mode | What `ecluse up` does | Best for |
|---|---|---|
| `container` | Runs all services in Docker (app + data) | Fully containerized stacks, devcontainer repos |
| `hybrid` | Runs data services in Docker, writes env — you start the app | Rails/Django/Node with a postgres+redis compose file |
| `host` | Writes env vars only — starts nothing | Pure native stacks with no Docker |

## container

Everything runs in Docker. ecluse generates a compose overlay that maps each service to a slot-specific port and names volumes per-slot. Nothing runs on the host outside of Docker.

Best when your repo already has a `docker-compose.yml` that runs the full stack.

## host

Your app runs natively. ecluse allocates ports and writes them to `.env.ecluse`. No containers are started. Hooks handle any setup (migrations, seeding).

Best when there are no data services or you manage them externally.

## hybrid

Data services (Postgres, Redis, etc.) run in containers with per-slot ports. Your app runs natively and connects to them via the ports in `.env.ecluse`.

Best when you have a compose file for data services but run the app with `npm run dev` or similar. See [Hybrid mode setup](hybrid-setup.md) for how to label your compose file.

## Changing modes

Mode is stored in `.ecluse.toml` and shared by all sessions. To change mode, edit `.ecluse.toml` and tear down any existing sessions first — running sessions use the mode that was active when they were created.
