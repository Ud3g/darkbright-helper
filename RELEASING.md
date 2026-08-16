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
5. Tag that commit and push the tag:

   ```bash
   git tag X.Y.Z
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

- Tags `0.7.1` and older predate this workflow and have no binaries; `0.8.0`
  was tagged retroactively (the workflow file does not exist at that commit,
  so no artifact was built for it).
