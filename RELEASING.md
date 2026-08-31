# Releasing

Releases are GitHub-only (nothing is published to crates.io — the crate's pub
surface is internal, enforced by `publish = false` in the manifest). A release is cut from `main` by pushing a bare-semver
tag (`0.8.0`, no `v` prefix — matches all existing tags), which triggers
`.github/workflows/release.yml`.

## Procedure

1. Verify `main` is green in CI.
2. Bump `version` in `Cargo.toml`, then run `cargo check` so `Cargo.lock`
   picks up the new version (CI builds `--locked` and would fail on a stale
   lockfile).
3. In `CHANGELOG.md`, retitle the `[Unreleased]` section to
   `[X.Y.Z] — YYYY-MM-DD` and start a fresh `[Unreleased]` above it. The
   dated heading and the tag belong together — never commit a dated version
   heading without also tagging it.
4. Commit (`chore: release X.Y.Z`), push, wait for CI to pass.
5. Tag that commit and push the tag. Tags are signed (`tag.gpgsign` is on),
   which makes them annotated — so a message is required and a bare
   `git tag X.Y.Z` fails with `fatal: no tag message?`:

   ```bash
   git tag -m "X.Y.Z" X.Y.Z
   git push origin X.Y.Z
   ```

6. The release workflow then runs on `windows-latest`: it checks that the tag
   matches the `Cargo.toml` version, runs the test suite, builds
   `--release --locked`, generates `THIRD-PARTY-NOTICES.html` with cargo-about
   (config in `about.toml` + `about.hbs`; a dependency license outside the
   accepted list fails the release), packages
   `darkbright-helper-X.Y.Z-windows-x64.zip` (exe + both `LICENSE-*` files +
   notices), attests build provenance, and creates the GitHub release with the
   zip attached. Release notes point at the CHANGELOG and carry the zip's
   SHA-256 plus the `gh attestation verify` command.

## Notes

- **The tag is the point of no return.** A repository ruleset blocks tag
  deletion and updates with no bypass, so a pushed tag cannot be removed or
  moved — not even by the owner. If the release workflow fails partway, that
  version number is spent: fix the cause and release the next patch version
  rather than retrying the same one. Anything cheap to verify beforehand is
  worth verifying, in particular that `cargo about generate --fail` succeeds,
  since a dependency licence outside the accepted list aborts the run after the
  tag already exists.
- **The tag is what makes a binary look released.** The version shown in the
  tray menu and the settings window is derived from `git describe`, so a build
  made anywhere other than a clean checkout of the tag appends a `(dev)` marker
  naming its commit. The release workflow therefore checks out with
  `fetch-depth: 0`; a shallow checkout without the tag would ship a release
  binary labelled as a dev build. Nothing to do during a release beyond leaving
  that setting alone — but it is why a locally built binary never claims to be
  the release.
- Tags `0.7.1` and older predate this workflow and have no binaries; `0.8.0`
  was tagged retroactively (the workflow file does not exist at that commit,
  so no artifact was built for it).
