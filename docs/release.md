# Release process

How to cut a release. Releases are built by GitHub Actions
(`.github/workflows/release.yml`) and appear on the repo's **Releases** page as a
single portable `.exe`. No installer, no code signing.

## What ships

- `valorant-lightweight-tracker.exe`, the portable single executable from
  `src-tauri/target/release/`. The filename carries no version on purpose, so it
  stays stable across releases (the auto-updater replaces the file in place); the
  version lives in the tag and the release name.
- `valorant-lightweight-tracker.exe.sha256` — the exe's SHA-256 as lowercase hex,
  and nothing else (no file name, no trailing newline). The auto-updater verifies
  the download against it before swapping anything.
- Built on `windows-latest` with `pnpm tauri build --no-bundle` (no NSIS/MSI
  bundles, no updater json).
- The release description is the matching `## vX.Y.Z` section of `CHANGELOG.md`,
  with the auto-generated commit list since the previous tag appended below it.

## Gates

The workflow triggers on any pushed `v*` tag, then refuses to publish unless all
of these pass. The first three run right after checkout, before any toolchain is
installed, so a bad tag fails within seconds:

1. **Validate tag format** — the tag must be exactly `vX.Y.Z` (no pre-release or
   build suffix, no leading zeroes). Anything else fails with the tag name in the
   message.
2. **Verify tag matches the manifest versions** — the tag version must equal the
   version in `package.json`, `src-tauri/Cargo.toml` (the `[package]` version)
   and `src-tauri/tauri.conf.json`. A mismatch lists every offending file; the
   fix is `pnpm bump` plus a re-tag.
3. **Extract the changelog entry** — `CHANGELOG.md` must contain a
   `## vX.Y.Z` section for the tag, and it must not be empty. The section body
   becomes the release description; a missing or empty one fails with "write the
   changelog entry for vX.Y.Z before tagging".
4. **Typecheck and build frontend** — `pnpm build` (`tsc && vite build`).
5. **Run Rust tests** — `cargo test --manifest-path src-tauri/Cargo.toml
   --locked`.

Only after those does the exe build and the release get created.

## Steps

Standard flow:

1. **Bump the version** — updates `package.json`, `src-tauri/Cargo.toml`,
   `src-tauri/tauri.conf.json` and this crate's entry in `src-tauri/Cargo.lock`
   together (the lock has to move too, or the `--locked` test gate fails):

   ```sh
   pnpm bump 0.2.0          # explicit version
   pnpm bump patch          # or: minor | major
   ```

   The script refuses to run if the four files currently disagree; fix them to
   match first. It does no git actions.

   Then add a `## v0.2.0 - YYYY-MM-DD` section at the top of `CHANGELOG.md`
   listing what changed. Write it for the people using the app, in everyday
   words ("Added", "Fixed", "Improved"), not in commit or module terms.

2. **Commit** the version bump and the changelog:

   ```sh
   git add package.json src-tauri/Cargo.toml src-tauri/tauri.conf.json src-tauri/Cargo.lock CHANGELOG.md
   git commit -m "Release v0.2.0"
   ```

3. **Tag** with a matching `vX.Y.Z` tag (the workflow rejects any other shape):

   ```sh
   git tag v0.2.0
   ```

4. **Push the commit and the tag:**

   ```sh
   git push
   git push origin v0.2.0
   ```

5. **Watch Actions** — the `Release` workflow runs the gates above, builds the
   exe and creates the GitHub Release. When it finishes, the release (with the
   attached `valorant-lightweight-tracker.exe`) appears on the Releases page.

To undo a bad tag before/while it builds: `git push origin :v0.2.0` deletes the
remote tag (delete the local one with `git tag -d v0.2.0`), then delete the draft
release if one was created.

## Auto-updater

Lives in `src-tauri/src/updater.rs` (two commands, `check_update` and
`apply_update`; see `docs/ipc-contract.md`). There is no updater plugin and no
signing key: the app talks to the GitHub API directly.

- **Check** — once per app start, and again whenever the version line on the
  waiting screen is clicked. It GETs
  `/repos/Superamaja/valorant-lightweight-tracker/releases/latest` and compares
  the tag against the compiled-in `CARGO_PKG_VERSION`, numerically, field by
  field. Pre-releases are excluded by the endpoint itself. The request is
  unauthenticated (60 per hour per IP), **so the repo and its releases must be
  public** for this to work at all. Being offline, rate-limited or handed an
  unexpected shape ends as a "Check failed" line, never a crash.
- **Install** — clicking the header's `Update: vX.Y.Z` chip. Both asset URLs come
  from one release record, so the exe and its checksum can never be from
  different releases. The exe streams to `…exe.new` beside the running one and is
  rejected unless its SHA-256 matches the `.sha256` asset. Then the Windows
  rename dance: the running `…exe` becomes `…exe.old` (a running binary can be
  renamed, not overwritten), `…exe.new` takes its name, and the new binary is
  spawned detached. The app only quits once that spawn succeeded; any failure
  before or after the renames rolls back, and the user is told to download the
  new version manually.
- **Cleanup** — every start deletes a leftover `…exe.old` (best effort, retried
  once, since the process that spawned this one may still be exiting).

## First-time setup

The workflow only works once the repo lives on GitHub:

- The repo must be pushed to GitHub (remote `origin` -> `Superamaja/valorant-lightweight-tracker`).
- **Actions must be enabled** for the repo (Settings -> Actions -> General).
- No secrets are needed — the workflow uses the automatic `GITHUB_TOKEN`, which
  already has the `contents: write` permission granted in the workflow.

## Verifying a build locally (optional)

You don't need to run the full release build to sanity-check, but if you want the
exact artifact the CI produces:

```sh
pnpm tauri build --no-bundle
# -> src-tauri/target/release/valorant-lightweight-tracker.exe
```
