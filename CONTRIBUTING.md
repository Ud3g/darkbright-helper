# Contributing

Thanks for considering it. Before you invest time, please read this — it will save us both
some.

## Before you start

- **Check [Scope](README.md#scope) first, and talk before you build.** For anything
  outside the current scope — the non-goals included — please open an issue *before*
  writing code. The constraint is not code quality: every merged feature becomes mine to
  maintain, on hardware I may not own, long after the PR. An excellent, self-contained
  implementation can absolutely change my mind; a finished PR landing unannounced is the
  worst starting position for that conversation, because by then the effort is already
  spent.
- **No timelines.** Reviews happen when they happen — see
  [Support and cadence](README.md#support-and-cadence). A PR sitting quietly is not a
  rejection, but I cannot promise it will ever be merged either.
- This project is **Windows-only** and largely LLM-generated under my direction (see
  [How this project was built](README.md#how-this-project-was-built)). I may be slow to
  judge subtle Rust; patches that come with a clear explanation of *why* they are correct
  have a much better chance.

## Building and testing

- **You need a Windows host** (or the `x86_64-pc-windows-msvc` target). `src/main.rs` and
  everything under `src/platform/windows/` will not compile on Linux/macOS hosts; the
  platform-agnostic logic in `src/core/` and `src/error.rs` does compile and test
  anywhere.
- MSRV is **Rust 1.88** (2024 edition).

Every PR must pass what CI enforces:

```bash
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings   # clippy all + pedantic are warn-by-default
cargo test
```

- Anything touching **DDC/CI, the OSD, the overlay, the tray, hotkeys, or power events**
  needs a manual test on real hardware — see "Integration Testing" in
  [docs/architecture.md](docs/architecture.md). Say in the PR what you tested on which
  setup; untested hardware-path changes will wait until I can verify them myself, which
  may take long.

## Code conventions

[docs/architecture.md](docs/architecture.md) is the source of truth for behaviour;
[docs/code-conventions.md](docs/code-conventions.md) covers FFI and style rules. The short
version:

- Keep `unsafe`/FFI isolated in `src/platform/windows/`, wrapped in RAII types; put
  testable logic in `src/core/`.
- Prefer the `Result`-returning `windows`-crate bindings; avoid `as` casts; document
  public items (`# Errors`/`# Panics` — clippy enforces this).
- Logging: structured key-value form, log at the point of handling, never log PII —
  monitor serials at `debug` only, and only via `MonitorId::full_identity()`.

## Commits and privacy

- **Please commit with your GitHub noreply address** (`<id>+<user>@users.noreply.github.com`,
  see [GitHub's email settings](https://github.com/settings/emails)). Git history is
  public and permanent; this repo deliberately contains no personal email addresses, and
  I would like to keep it that way — for your sake.
- Keep commit messages terse; a `feat:`/`fix:`/`docs:` prefix matches the existing
  history.

## Licensing

By submitting a contribution you agree it is dual-licensed under Apache-2.0 and MIT, as
described in the [License](README.md#license) section — no CLA, no extra paperwork.
