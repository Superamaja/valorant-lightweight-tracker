# Project Context

Last updated: 2026-08-24

## Goal

A lightweight Windows desktop app with a good UI that shows an in-match player table for the user's current Valorant match — like VALORANT-rank-yoinker (vRY), but a real GUI instead of a console.

## Decisions made

| Decision | Choice | Why |
|---|---|---|
| Backend approach | Build our own against Riot's local client API directly (option 3) — no vRY code dependency | vRY is a console app, not a library; the plumbing we need is small (a few hundred lines); endpoints change rarely |
| Correctness reference | vRY source code | User verified vRY's data is accurate in real matches |
| ValoTracker | Do not reuse its backend logic | User verified its data is incorrect |
| Static data (icons, rank/skin/agent names, images) | valorant-api.com | Free, no key, same source vRY uses |
| Scope v1 | In-match player table only | User request |
| Discord RPC | Not now, keep architecture open for it later | User request |
| Tech stack | **DECIDED: Tauri 2 + React + TypeScript + Vite + Tailwind CSS v4** | Small native footprint, good UI story, single-language plumbing behind the Rust backend |

## Architecture intent (regardless of stack)

- Thin native/backend layer: read Riot lockfile, call local client HTTPS endpoints (self-signed cert, basic auth) + remote pd/glz endpoints, listen on local websocket for match state.
- All interesting logic (presence parsing, tier→rank mapping, table assembly) lives in one language behind an adapter interface so the plumbing is swappable.
- UI reads from that layer; keep a seam where Discord RPC could plug in later.

## Status / next steps

1. ~~User picks stack (Tauri vs Python).~~ Done — Tauri 2.
2. ~~Scaffold project.~~ Done — see Repo layout below.
3. Implement lockfile + local API auth, then presence → player list → ranks pipeline, cross-checking values against vRY.

## Repo layout

Scaffolded with `pnpm create tauri-app` (react-ts template), files at the repo root.

```
.
├── index.html            # Vite entry HTML (window title set here)
├── package.json          # Frontend deps + scripts (pnpm)
├── pnpm-workspace.yaml    # pnpm settings (allows esbuild build script)
├── vite.config.ts         # Vite config; React + @tailwindcss/vite plugins
├── tsconfig.json
├── public/                # Static assets served as-is
├── src/                   # React + TypeScript frontend
│   ├── main.tsx           # React entry, imports index.css
│   ├── App.tsx            # Placeholder page (dark bg, title, empty player-table region)
│   └── index.css          # Tailwind v4 entry (`@import "tailwindcss";`)
└── src-tauri/             # Rust / Tauri 2 backend (minimal default scaffold)
    ├── Cargo.toml
    ├── tauri.conf.json    # productName, identifier com.connor.valorant-tracker, 1000x700 window
    └── src/
        ├── main.rs        # Calls valorant_lightweight_tracker_lib::run()
        └── lib.rs         # Default `greet` command only; no Riot logic yet
```

Frontend styling uses Tailwind CSS v4 via the Vite plugin (`@tailwindcss/vite`) — no
`tailwind.config.js` or PostCSS needed; utilities come from `@import "tailwindcss";` in
`src/index.css`.

### How to run

- `pnpm install` — install frontend deps (first run also compiles the esbuild binary).
- `pnpm tauri dev` — run the desktop app in dev (spins up Vite + the Rust shell with hot reload).
- `pnpm tauri build` — produce a release build + installer.
- `pnpm build` — frontend-only typecheck (`tsc`) + Vite production build (output in `dist/`).
- `cd src-tauri && cargo check` — typecheck the Rust side.

Note: `cargo`/`rustc` are not on PATH by default in this environment; prepend the rustup
cargo bin dir first (PowerShell: `$env:Path = "$env:USERPROFILE\scoop\apps\rustup\current\.cargo\bin;$env:Path"`).

## Conventions

- Python work: always in a venv. Node work: pnpm.
- Keep docs/ updated whenever decisions or scope change — this folder is the cross-session memory.
