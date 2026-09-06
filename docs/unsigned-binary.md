# Running an unsigned binary

The release binaries of darkbright-helper are **not code-signed**. This page collects
everything that follows from that: the prompts you will see, why they keep coming back, the
antivirus false positives that unsigned native executables attract, and what you can verify
instead of trusting the file blindly.

The short version lives in the [README](../README.md#running-an-unsigned-binary); this is
the long one, for when a warning has actually stopped you.

## The prompts, and how to get past them

Your browser warns on download (Edge:
**Keep → Show more → Keep anyway**), and Windows then shows **"Windows protected your PC"**
the first time you run each new version — proceed with **More info → Run anyway**.
"Unrecognized" is not "malicious".

You can also skip the prompt entirely by clearing the download mark on the `.zip` *before*
extracting it: right-click it → **Properties** → tick **Unblock** → OK (or `Unblock-File`
in PowerShell) — files extracted afterwards carry no mark. On managed corporate machines an
administrator may have removed the "Run anyway" option altogether; there, building from
source is the way out, since locally built binaries carry no download mark and SmartScreen
does not apply to them.

## Why it keeps coming back

SmartScreen reputation attaches to the specific file, and an
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

## Antivirus false positives

Unsigned, low-download-count native executables are a known
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

## Smart App Control (Windows 11)

SAC blocks unsigned code that has no established
reputation, and unlike SmartScreen it offers no per-app override. Note that **building from
source does not help here**: SAC checks signatures on all executable code the loader
touches, not just downloaded files, so a locally compiled unsigned binary is blocked just
the same. Most people are unaffected — SAC only engages after an evaluation period on a
clean install, and Windows turns it off automatically on machines that look like developer
workstations. If you do have it on, the options are to turn it off (**Settings → Windows
Security → App & Browser Control → Smart App Control**) or to wait for a signed release;
since March 2026 it can be toggled without a clean install.

## What you can verify

Every release is built by the
[release workflow](../.github/workflows/release.yml) from a tagged commit in this repository.
The release notes carry the zip's SHA-256 (compare with `Get-FileHash`), and both the zip
and the exe inside it have a signed build-provenance attestation:

```powershell
gh attestation verify .\darkbright-helper-<version>-windows-x64.zip --repo Ud3g/darkbright-helper
```

proves the artifact was built by this repository's workflow from a specific commit. Since
you are being asked to click past a security warning, these checks are the meaningful step.

