# rails-monorepo

Rails 7 app with Sidekiq background jobs and a Blazer admin panel, running in hybrid mode.

Data services (Postgres, Redis) run in Docker with per-slot offset ports. Three Rails processes run natively — each on its own dedicated port from the `[ports]` table.

## Processes

| Process | Started by | Port env var | Description |
|---------|-----------|--------------|-------------|
| Puma | `bin/rails server` | `ECLUSE_WEB_PORT` | Main Rails app |
| Sidekiq | `bundle exec sidekiq` | — | Background job processor (no UI port needed) |
| Blazer | `bin/rails server -p $ECLUSE_ADMIN_PORT` | `ECLUSE_ADMIN_PORT` | SQL-based admin dashboard |

Sidekiq Web UI is mounted inside the main Rails app at `/sidekiq` (routed through Puma on `ECLUSE_WEB_PORT`). The `ECLUSE_SIDEKIQ_PORT` is reserved for a standalone Sidekiq Web rack process if you prefer to run it separately.

## Ports (slot 1 example)

| Variable               | Port | Service              |
|------------------------|------|----------------------|
| `ECLUSE_WEB_PORT`      | 3100 | Rails Puma (+ PORT alias) |
| `ECLUSE_SIDEKIQ_PORT`  | 3101 | Sidekiq Web (standalone) |
| `ECLUSE_ADMIN_PORT`    | 3102 | Blazer admin panel   |
| `ECLUSE_POSTGRES_PORT` | 5532 | Postgres (compose)   |
| `ECLUSE_REDIS_PORT`    | 6479 | Redis (compose)      |

## Usage

```sh
ecluse up my-feature
ecluse shell my-feature

# Inside the session shell
bin/dev   # starts all processes via Procfile.dev (foreman/overmind)

# Or individually:
# bin/rails server -p $ECLUSE_WEB_PORT
# bundle exec sidekiq
# bin/rails server -p $ECLUSE_ADMIN_PORT -P tmp/pids/admin.pid

ecluse down my-feature
```

## App config

Rails reads port from the env var at boot — no hardcoded 3000:

```ruby
# config/puma.rb
port ENV.fetch("ECLUSE_WEB_PORT", 3000)
```

Sidekiq connects to Redis via the offset port:

```yaml
# config/sidekiq.yml
:redis_url: redis://localhost:<%= ENV["ECLUSE_REDIS_PORT"] || 6379 %>
```
