# mongo-hybrid

Node.js API with MongoDB, running in hybrid mode.

MongoDB runs in a Docker container managed by ecluse. The Node.js process runs natively. Each worktree gets its own MongoDB instance on an offset port.

Note: MongoDB is not a Postgres-compatible service, so ecluse does not set `DATABASE_URL`. Instead it sets `ECLUSE_MONGODB_PORT` with the offset port for this slot. Your application constructs the connection string from that variable.

## Mode

`hybrid` — MongoDB containerized, Node.js runs natively.

## Services

| Service  | Role        | Label              |
|----------|-------------|--------------------|
| mongodb  | data        | —                  |
| web      | app (Node)  | `ecluse.role: app` |

## Environment variables set by ecluse

| Variable               | Description                              |
|------------------------|------------------------------------------|
| `ECLUSE_SLUG`          | Session slug                             |
| `PORT`                 | App port (`base_port + slot`, e.g. 3001 for slot 1) |
| `ECLUSE_MONGO_PORT`    | Per-slot host port for MongoDB           |

## Constructing the MongoDB connection string

In your application:

```ts
const mongoUrl = `mongodb://localhost:${process.env.ECLUSE_MONGODB_PORT}/${process.env.ECLUSE_SLUG}`;
```

## Hooks

- `on_up`: runs `npm run db:seed` (optional) to seed initial data for this slot.

## Usage

```sh
ecluse init
ecluse up my-feature
ecluse shell my-feature

# Inside the session shell
npm run dev

ecluse down my-feature
```
