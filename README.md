# RustGo

RustGo is a minimal [go](<https://en.wikipedia.org/wiki/Go_(game)>) platform.

The engine is written in Rust with Chinese rules and 7.5 komi.

- `rustgo-core`: the game engine
- `rustgo-server`: a Cloudflare Worker hosting each room in a Durable Object with its own SQLite database
- `web`: the frontend, deployed to Vercel

Board size, komi and handicap are chosen per game when it is created.

## Setup

```sh
rustup target add wasm32-unknown-unknown
cargo install worker-build
cd rustgo-server && bun install && bun run dev  # backend on 8787
cd web && bun install && bun run dev            # frontend on 5173
```

## Configuration

`ALLOWED_ORIGINS` in `rustgo-server/wrangler.toml` lists the comma-separated origins of the frontend. Unset, browsers on other sites cannot call the API.

The frontend reads `VITE_API_URL` at build time; unset, it talks to the site it is served from.

## Deployment

1. Set `ALLOWED_ORIGINS` in `rustgo-server/wrangler.toml` to the site's origin, `https://rustgo.xyz`, then run `bun run deploy` in `rustgo-server` after `bunx wrangler login`. `wrangler deploy` prints the Worker's URL.
2. Import the repository on Vercel with `web` as the root directory, which picks up `web/vercel.json`. Set `VITE_API_URL` to the Worker's URL and redeploy whenever it changes.

Rooms hibernate between messages while players stay connected, and games survive restarts and deploys (deploys do disconnect everyone briefly). Games are never deleted yet.
