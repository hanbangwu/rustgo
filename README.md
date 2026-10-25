# igo

Igo (stylized as igo) is a minimal [go](<https://en.wikipedia.org/wiki/Go_(game)>) platform.

The engine is written in Rust with Chinese rules and 7.5 komi.

## Setup

```
cargo run -p igo-server               # backend on 3030
cd web && bun install && bun run dev  # frontend on 5173
```

Games are kept in memory only, unless `DATABASE` points at a SQLite file:

```
DATABASE=igo.db cargo run -p igo-server
```

## Configuration

| Variable          | Default     | Meaning                                                      |
| ----------------- | ----------- | ------------------------------------------------------------ |
| `PORT`            | `3030`      | Port to listen on                                            |
| `DATABASE`        | unset       | SQLite file that keeps games across restarts                 |
| `EXPIRY_DAYS`     | `1`         | Delete a game after this many days with nobody using it      |
| `MAX_ROOMS`       | `10000`     | Refuse to create new games beyond this many in memory        |
| `STATIC_DIR`      | `web/build` | Built frontend to serve                                      |
| `ALLOWED_ORIGINS` | unset       | Comma-separated origins of a frontend hosted on another site |

The frontend reads `VITE_API_URL` at build time; unset, it talks to the site it is served from.

Board size, komi and handicap are chosen per game when it is created.

## Deployment

```
docker build -t igo .
docker run --rm -p 3030:3030 -v igo-data:/data igo
```

The image stores games in `/data/igo.db`; mount a volume there to keep them across redeploys.
Run a single instance: rooms live in that process's memory.

### Vercel and Render

The frontend can go on Vercel and the backend on Render's free plan:

1. Create a Render Blueprint from this repository. It reads `render.yaml` and builds `Dockerfile.render`.
   Set `ALLOWED_ORIGINS` to the Vercel site's origin.
2. Import the repository on Vercel with `web` as the root directory, which picks up `web/vercel.json`.
   Set `VITE_API_URL` to the Render service's URL and redeploy whenever it changes.

A free Render service has no disk, so games last only while the process does.
It sleeps after 15 minutes without traffic and takes about a minute to wake;
open games send a heartbeat so it stays up while anyone is playing.
