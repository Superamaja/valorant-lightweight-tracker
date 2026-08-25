# Valorant Lightweight Tracker

A lightweight Windows desktop app that shows an **in-match player table** for
your current VALORANT match: everyone's rank, RR, peak rank, level, agent, and
per-player stats in a clean, image-first GUI instead of a console window.

<!-- App screenshot goes here once ready:
![Valorant Lightweight Tracker showing an in-match player table](docs/assets/app-screenshot.png)
-->

## Download

Grab the latest portable build from the
[**Releases page**](https://github.com/Superamaja/valorant-lightweight-tracker/releases):

1. Download `valorant-lightweight-tracker-vX.Y.Z.exe`.
2. Run it. There is **no installer**; the single `.exe` is the whole app.

### "Windows protected your PC" (SmartScreen)

The exe is unsigned, so Windows SmartScreen may warn you the first time you run
it. Click **More info -> Run anyway**. (This is expected for small unsigned
apps; the source is in this repo if you'd rather build it yourself.)

## Requirements

- **Windows 10 or 11**
- **WebView2 runtime**, preinstalled on current Windows. If missing, get it
  from Microsoft's [Evergreen WebView2](https://developer.microsoft.com/microsoft-edge/webview2/)
  page.
- **VALORANT running**. The app reads your local Riot client. Launch the game
  (menu is enough), then start the tracker; it will wait until a match begins.

## Build from source

Requires [Node.js](https://nodejs.org/) + [pnpm](https://pnpm.io/) and the
[Rust toolchain](https://www.rust-lang.org/tools/install) (Tauri 2
prerequisites).

```sh
pnpm install                    # install frontend dependencies
pnpm tauri dev                  # run the app in development (hot reload)
pnpm tauri build --no-bundle    # build the portable exe
```

The exe lands in `src-tauri/target/release/`. This is the same build the
release workflow runs; see [`docs/release.md`](docs/release.md) for how tagged
releases are built.

## Development

The UI can run against JSON snapshot data instead of a live game, including in
a plain browser with no Rust or Valorant at all. Debug builds can also capture
real snapshots to JSON while you play. See [`docs/testing.md`](docs/testing.md)
for both workflows.

## Disclaimer

This project is **not affiliated with, endorsed, sponsored, or approved by Riot
Games**. VALORANT and Riot Games are trademarks or registered trademarks of Riot
Games, Inc. It uses Riot's unofficial local client API, which can change at any
time. **Use at your own risk.**

## Credits

- [**VALORANT-rank-yoinker**](https://github.com/mdevio/VALORANT-rank-yoinker)
  (ISC), the reference implementation for data correctness. This app builds its
  own backend but follows vRY's approach to the Riot endpoints.
- [**valorant-api.com**](https://valorant-api.com/), a free, no-key source for
  all static assets (agent, rank, and skin names and imagery).
