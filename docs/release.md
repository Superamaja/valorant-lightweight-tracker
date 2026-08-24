# Release process

How to cut a release. Releases are built by GitHub Actions
(`.github/workflows/release.yml`) and appear on the repo's **Releases** page as a
single portable `.exe`. No installer, no auto-updater, no code signing.

## What ships

- One artifact per release: `valorant-lightweight-tracker-v<version>.exe`, the
  portable single executable from `src-tauri/target/release/`.
- Built on `windows-latest` with `pnpm tauri build --no-bundle` (no NSIS/MSI
  bundles, no updater json).
- Release notes are auto-generated from the commits since the previous tag.

## Steps

The workflow triggers on a pushed tag matching `v*`. The version in the tag
should match the version in the three synced files (kept in step by the bump
script). Standard flow:

1. **Bump the version** — updates `package.json`, `src-tauri/Cargo.toml`, and
   `src-tauri/tauri.conf.json` together:

   ```sh
   pnpm bump 0.2.0          # explicit version
   pnpm bump patch          # or: minor | major
   ```

   The script refuses to run if the three files currently disagree; fix them to
   match first. It does no git actions.

2. **Commit** the version bump:

   ```sh
   git add package.json src-tauri/Cargo.toml src-tauri/tauri.conf.json
   git commit -m "Release v0.2.0"
   ```

3. **Tag** with a matching `v`-prefixed tag:

   ```sh
   git tag v0.2.0
   ```

4. **Push the commit and the tag:**

   ```sh
   git push
   git push origin v0.2.0
   ```

5. **Watch Actions** — the `Release` workflow builds the exe and creates the
   GitHub Release. When it finishes, the release (with the attached
   `valorant-lightweight-tracker-v0.2.0.exe`) appears on the Releases page.

To undo a bad tag before/while it builds: `git push origin :v0.2.0` deletes the
remote tag (delete the local one with `git tag -d v0.2.0`), then delete the draft
release if one was created.

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
