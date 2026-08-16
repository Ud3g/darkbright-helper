# darkbright-helper

## Overview
A hotkey-driven brightness adjustment tool for Windows.

**Platform:** Windows only. The core logic in `src/core/` is deliberately kept
platform-agnostic, so a Linux port would be structurally feasible — but it is **not planned
and not promised**. I may look into it if there is real demand and I have the time and
inclination; equally, it may never happen.

## Language
Rust (2024 edition) — chosen for cross-platform portability, low resource usage, and native Windows API integration (`windows` crate).

## Features

### Brightness Below Hardware Minimum
- Use a black fullscreen overlay with variable opacity
- Allows "dimming" below what the monitor natively supports
- *Note: Does not cover exclusive fullscreen games or certain Windows system UI (Taskbar, Start Menu).*

### Brightness Above Hardware Minimum
- Communicate with monitor via DDC/CI protocol
- Adjust VCP code `0x10` (brightness) directly

### Multi-Monitor Support
- Per-monitor control
- Hotkeys affect the monitor where the mouse pointer currently resides

### User Interface
- OSD overlay (similar to Windows volume indicator)
- Visual feedback on brightness changes
- System tray icon with context menu: live per-monitor status, Usage, Settings, Open Log Folder, Quit — plus warning entries and an icon badge while degraded (e.g. DDC unavailable)

## Scope

A deliberately narrow hobby project: adjust monitor brightness from the keyboard, on
Windows, including below the hardware minimum. That is all it currently tries to do.

**Non-goals** — these are settled, not open questions:

- Colour temperature, night light, or monitor gamma control
- Per-application or scheduled brightness profiles
- Telemetry, auto-update, or any network feature

**Not planned, but not ruled out:**

- A Linux port (see Platform, above)
- A settings GUI
- Some form of contrast handling to improve text readability at very low overlay levels
- Laptop internal panels, which do not speak DDC/CI and would need a separate Windows
  backend alongside it

None of these is promised, none has a timeline, and none is worth waiting for.

If you need something this tool does not do, forking is genuinely encouraged — the licence
permits it, and I would rather you have the tool you want than wait on me.

## Hotkeys
- **Primary**: `Ctrl+Shift+Up` / `Ctrl+Shift+Down` (reliable cross-keyboard default)
- **Secondary**: Dedicated brightness keys (`VK_BRIGHTNESS_UP/DOWN`) registered opportunistically
- Fully configurable via `config.json` (in `%APPDATA%`)

## Installation & Build

### Download

Prebuilt Windows binaries are published on
[GitHub Releases](https://github.com/Ud3g/darkbright-helper/releases)
(releases after 0.8.0), as a zip bundling the executable with its license files and
third-party notices. The binaries are not code-signed, so Windows will warn the first time
you run one — see [Running an unsigned binary](#running-an-unsigned-binary) for what to
expect and why. Alternatively, build from source as described below.

### Prerequisites
- Rust 1.88+ (2024 edition)
- Windows 10 or 11

### Build
```bash
git clone https://github.com/Ud3g/darkbright-helper.git
cd darkbright-helper
cargo build --release
```
The executable will be at `target/release/darkbright-helper.exe`.

### Debug vs Release Builds

| Build Type | Command | Console Window | Use Case |
|------------|---------|----------------|----------|
| **Debug** | `cargo build` | ✅ Visible | Development, viewing log output |
| **Release** | `cargo build --release` | ❌ Hidden | End-user distribution |

- **Debug builds** show a console window where log messages appear (controlled by `RUST_LOG` environment variable)
- **Release builds** use `windows_subsystem = "windows"` to hide the console, providing a clean GUI-only experience
- To diagnose a release build, enable the opt-in file log — see [Logging](#logging)

## Configuration

The configuration file is automatically created at:
`%APPDATA%\BrightnessControl\config.json`

### Default Configuration
```json
{
  "version": 1,
  "hotkeys": {
    "brightness_up": "Ctrl+Shift+Up",
    "brightness_down": "Ctrl+Shift+Down",
    "intercept_brightness_keys": false
  },
  "osd": {
    "timeout_ms": 1000,
    "opacity": 1.0
  },
  "brightness": {
    "step_percent": 5
  },
  "monitors": {},
  "refresh": {
    "periodic_seconds": 60,
    "inactivity_seconds": 30
  },
  "logging": {
    "file_enabled": false,
    "file_level": "info"
  }
}
```

### Options
- **hotkeys.brightness_up/down**: Combination strings (e.g., "Alt+F1", "Ctrl+Shift+Plus").
- **hotkeys.intercept_brightness_keys**: Enable low-level keyboard hook to capture dedicated brightness keys (default: false). See [Brightness Key Limitations](#brightness-key-limitations) for compatibility information.
- **osd.timeout_ms**: How long the OSD remains visible (100-10000 ms).
- **osd.opacity**: OSD window transparency (0.1-1.0).
- **brightness.step_percent**: Amount to change per keypress (1-50%).
- **refresh.periodic_seconds**: Background refresh interval to resync with external changes (0-3600, 0 = disabled).
- **refresh.inactivity_seconds**: Refresh before adjustment if inactive for this duration (0-600, 0 = disabled).
- **logging.file_enabled**: Opt-in rolling file log for release diagnostics (default: false). See [Logging](#logging).
- **logging.file_level**: Level filter for the file log — `error`/`warn`/`info`/`debug`/`trace` (default: info).

The `monitors` field is reserved for future per-monitor settings and currently ignored.

## Logging

- **Debug builds** log to the visible console; the level is controlled by `RUST_LOG` (default: debug).
- **Release builds** hide the console. For diagnostics, set `logging.file_enabled: true`: every log record is then also written to `%APPDATA%\BrightnessControl\darkbright.log`, reachable via the tray menu's "Open Log Folder". The file is size-capped: at 1 MB it rotates to `darkbright.log.old`, bounding disk use at ~2 MB while recent history survives.
- `logging.file_level` filters the file independently of the console (`RUST_LOG` does not affect the file). At `debug` and below the file contains monitor serial numbers and absolute paths — fine for a deliberately created diagnostic artifact, but worth knowing before sharing it.
- Crashes leave a trace: panics are logged (message + source location) and flushed to the file log before the process dies.

## Privacy

The tool performs no network I/O whatsoever — no telemetry, no update checks, no crash
reporting. It opens no sockets, and nothing in its dependency tree is capable of doing so.
The only files it touches are its own, in `%APPDATA%\BrightnessControl\`: `config.json`
(plus a `config.json.bak` mirror) and, when enabled, `darkbright.log`.

Two caveats worth stating plainly. That folder is *Roaming* AppData, so on a machine with
roaming profiles or folder redirection, Windows may sync it to a network share — that is
Windows rather than this tool, but it is the one way these files can leave your machine.
And if the process ever crashes, Windows Error Reporting may offer to send a report to
Microsoft, as it does for any program.

The file log is off by default and defaults to `info` level, at which it records your
monitors' manufacturer and model names but no serial numbers and no file paths. Raising
`logging.file_level` to `debug` or `trace` adds monitor serial numbers and absolute paths
containing your Windows user name — fine for a diagnostic session you started deliberately,
worth a glance before you attach the file to a bug report (see [Logging](#logging)).

## Usage

1. Run `darkbright-helper.exe`.
2. Use `Ctrl+Shift+Up` to increase brightness.
3. Use `Ctrl+Shift+Down` to decrease brightness.
4. If brightness reaches 0%, continuing to decrease will activate the dimming overlay.
5. Right-click the system tray icon to see per-monitor status and access Usage, Settings, the log folder, or Quit.

## Brightness Key Limitations

The `intercept_brightness_keys` option attempts to capture dedicated brightness keys (`VK_BRIGHTNESS_UP`/`VK_BRIGHTNESS_DOWN`) using a low-level keyboard hook.

**This feature only works on keyboards that send brightness keys through the standard Windows keyboard input path.**

| Keyboard Type | Works? | Reason |
|---------------|--------|--------|
| Most laptop built-in keyboards | ❌ No | Keys handled by firmware/ACPI before reaching Windows |
| Some external USB keyboards | ✅ Yes | Keys sent as standard HID key codes |
| Gaming keyboards with media keys | ⚠️ Maybe | Depends on manufacturer implementation |

**If your brightness keys don't work with this option:**
- Your keyboard's brightness keys are intercepted by firmware or a dedicated driver before Windows sees them
- The native Windows brightness OSD will still appear regardless of this setting
- Use the primary hotkeys (`Ctrl+Shift+Up/Down`) instead

**Notes:**
- Some antivirus software may flag low-level keyboard hooks as suspicious behavior
- Disabled by default to avoid false positives for users who don't need the feature

## Running an unsigned binary

The release binaries are **not code-signed**. Your browser warns on download (Edge:
**Keep → Show more → Keep anyway**), and Windows then shows **"Windows protected your PC"**
the first time you run each new version — proceed with **More info → Run anyway**.
"Unrecognized" is not "malicious".

You can also skip the prompt entirely by clearing the download mark on the `.zip` *before*
extracting it: right-click it → **Properties** → tick **Unblock** → OK (or `Unblock-File`
in PowerShell) — files extracted afterwards carry no mark. On managed corporate machines an
administrator may have removed the "Run anyway" option altogether; there, building from
source is the way out, since locally built binaries carry no download mark and SmartScreen
does not apply to them.

**Why it keeps coming back.** SmartScreen reputation attaches to the specific file, and an
unsigned binary starts from zero with every release. Microsoft's own wording: "Unsigned
files must build reputation anew with every update", reputation accrues only through
download volume ("several weeks and hundreds of clean installs"), and there is "no need (or
mechanism) to manually submit a file for SmartScreen reputation review". For a project this
size, the prompt is effectively permanent.

Signing *would* help — not by removing the first-run prompt, but by letting reputation
carry across releases and by showing a verified publisher name instead of "Unknown
publisher". An **EV** certificate specifically would not help: since August 2024 Microsoft
removed all EV Code Signing OIDs from its Trusted Root Program and treats all code-signing
certificates equally. Signing is something I may set up later — SignPath Foundation offers
it free to qualifying open-source projects — but it is not in place today.

**Antivirus false positives.** Unsigned, low-download-count native executables are a known
source of machine-learning false positives in Microsoft Defender — typically
`Trojan:*/Wacatac.*!ml`, where the `!ml` suffix marks a heuristic guess rather than a
signature match. Rust-built CLI tools get caught by this regularly (see e.g.
[tauri-apps/tauri#2486](https://github.com/tauri-apps/tauri/issues/2486)); it is not
specific to Rust, and it is not fixed. The optional `intercept_brightness_keys` low-level
keyboard hook makes a flag more likely, which is one reason it is off by default. If you
hit one, please open an issue and I will submit it to Microsoft as a developer report — or
submit it yourself at Microsoft's
[file submission portal](https://www.microsoft.com/en-us/wdsi/filesubmission) as a **Home
customer** (a Microsoft account is required).

**Smart App Control (Windows 11).** SAC blocks unsigned code that has no established
reputation, and unlike SmartScreen it offers no per-app override. Note that **building from
source does not help here**: SAC checks signatures on all executable code the loader
touches, not just downloaded files, so a locally compiled unsigned binary is blocked just
the same. Most people are unaffected — SAC only engages after an evaluation period on a
clean install, and Windows turns it off automatically on machines that look like developer
workstations. If you do have it on, the options are to turn it off (**Settings → Windows
Security → App & Browser Control → Smart App Control**) or to wait for a signed release;
since March 2026 it can be toggled without a clean install.

**What you can verify.** Every release is built by the
[release workflow](.github/workflows/release.yml) from a tagged commit in this repository.
The release notes carry the zip's SHA-256 (compare with `Get-FileHash`), and both the zip
and the exe inside it have a signed build-provenance attestation:

```powershell
gh attestation verify .\darkbright-helper-<version>-windows-x64.zip --repo Ud3g/darkbright-helper
```

proves the artifact was built by this repository's workflow from a specific commit. Since
you are being asked to click past a security warning, these checks are the meaningful step.

## Support and cadence

If you run into trouble, I will generally take a look. The most useful thing you can do is
enable the file log (`logging.file_enabled: true`, with `logging.file_level` set to
`debug`), reproduce the problem, and attach the log to an issue — that is what I need in
order to investigate anything. Please skim it first: at `debug` level it contains your
monitors' serial numbers and absolute paths.

What I cannot offer is any commitment on timing. I have a full-time job and children, and
this is a spare-time project. There is no response-time target of any kind. Long quiet
periods are normal and do not mean the project is abandoned — but if more comes in than I
expect, it is equally possible that an issue sits for a very long time, or that nothing
happens at all. I would rather say that plainly than let you infer a promise I cannot keep.

Hardware-specific DDC/CI problems are the hardest case: monitors might misbehave in ways I
cannot reproduce on my own hardware, and some of those I will close without a fix.

## How this project was built

The code in this repository is LLM-generated. I directed the work — requirements,
architecture decisions, design review, and testing against real hardware — but I did not
hand-write the Rust. I am not a Rust programmer, and I would rather say so than pretend
otherwise.

What that means in practice:

- **It is reviewed, not dumped.** The project has been through documented architecture
  reviews (see `docs/`), with findings tracked and resolved rather than waved through.
  DDC/CI behaviour is verified manually against real monitors, because it cannot be
  meaningfully unit-tested.
- **The commit history says so.** Most commits carry `Co-authored-by` trailers naming the
  model that produced them.
- **My support depth is limited by this.** I can reason about this codebase's design and
  behaviour, but I am not the right person to ask about advanced Rust idioms, and I may be
  slow to judge a subtle patch. Factor that in before depending on this tool.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

The application icon (`res/icon.ico`, `res/icon.png`) is AI-generated; no copyright is
claimed over it.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for
inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed
as above, without any additional terms or conditions.
