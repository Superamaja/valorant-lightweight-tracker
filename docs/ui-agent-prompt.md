# UI Agent Prompt

Ready-to-paste prompt for the user-run UI implementation agent. Keep in sync with ui-spec.md and ipc-contract.md if they change before the UI is built.

---

Implement the frontend UI of the Tauri 2 app at C:\users\conno\github\valorant-lightweight-tracker (Windows). The Rust backend is complete, reviewed, and live-verified — do NOT modify src-tauri/ (Rust), Cargo.toml, or tauri.conf.json. Your scope is src/ (React + TypeScript + Vite + Tailwind CSS v4, already wired), index.html, and frontend package deps (pnpm only).

Read these first, in order:
1. CLAUDE.md — project hard rules.
2. docs/ui-spec.md — THE design contract: layout, style, image-over-text principle, behaviors. Follow it exactly.
3. docs/ipc-contract.md — THE data contract: the `tracker-state` event, `start_tracker` / `get_tracker_state` commands, full TrackerSnapshot/PlayerRow field reference, ordering guarantee, and the two-phase loading behavior.
4. docs/project-context.md — background.

Build:
1. **Wiring**: on mount, `invoke("start_tracker")` then `listen("tracker-state", ...)`; also `invoke("get_tracker_state")` once for the current snapshot. Use `@tauri-apps/api` (installed). Type the payload exactly per ipc-contract.md.
2. **The player table** (the whole app): two team blocks — players[] arrives pre-ordered (own team first, self first); split on `isAlly`. Ally block ALWAYS blue-tinted, enemy red-tinted; NEVER color by the raw `team` field. Columns: agent portrait, name#tag, Vandal skin, Phantom skin, current rank icon + RR, peak rank icon (smaller, muted), HS%, WR (percent + games), last-5 W/L pips, RR delta, account level. Icons/portraits come as URLs in the payload — render images, tooltips for names (image-over-text is the core principle; rank NAMES are tooltips only). Party members: matching colored dots per partyId (players with a shared partyId, excluding solo).
3. **Two-phase loading**: the first snapshot of a match has heavy fields empty (`rrChange` null, `recentResults` [], `headshotPercent` null, skins null) — render subtle per-cell skeletons/placeholders, filled by the enriched snapshot that follows. Never block the table on them.
4. **States**: design the non-match states properly (most-seen screens): `ValorantNotRunning` ("Waiting for Valorant" + subtle pulse), `Menus`, `Pregame` (ally-only table, agent-select feel), `Ingame`. Header strip: map + mode + state chip + "last updated".
5. **Incognito**: `name` null + `incognito` true → show agent name as the display name (muted style), no tracker.gg link, no level.
6. **tracker.gg click**: player name click opens `https://tracker.gg/valorant/profile/riot/{encodeURIComponent(name#tag)}/overview` via `@tauri-apps/plugin-opener` (Rust plugin already registered; add the JS package with pnpm). Right-click or small icon: copy name#tag. Disabled for incognito.
7. **Style**: dark only, near-black bg, one red/coral accent, restrained — whitespace and the game imagery do the work. No light theme, no settings screen, no extra pages.
8. **Self row**: subtle highlight so the user finds themselves instantly.

Verify: `pnpm build` (tsc + vite) passes; then run `pnpm tauri dev` (prepend `$env:USERPROFILE\scoop\apps\rustup\current\.cargo\bin` to PATH in the shell) and confirm the window opens and shows the correct state screen (with Valorant closed you should see "Waiting for Valorant", NOT an error). Do not git commit — a Fable code review happens after.

Return: what you built, component structure, verification results, and any ipc-contract ambiguities you hit (flag, don't guess).

---
