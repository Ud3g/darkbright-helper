# Security Policy

## Reporting a vulnerability

Please use **GitHub's private vulnerability reporting** for this repository
(Security tab → *Report a vulnerability*). Do not open a public issue for anything you
believe is exploitable — a public issue is a disclosure to everyone at once.

I read reports and will respond, but this is a spare-time project by a single person:
**there is no response-time commitment of any kind** (see
[Support and cadence](README.md#support-and-cadence)).

## Supported versions

Only the **latest release** receives fixes. There are no backports.

## Scope — what a vulnerability looks like here

This tool has a deliberately small attack surface: it performs **no network I/O**, runs
without elevation, and talks only to local monitors (DDC/CI), the local config file, and
the Win32 UI. Things that would genuinely interest me:

- Memory-safety issues in the FFI layer (`src/platform/windows/`) that are reachable from
  untrusted input — e.g. a malformed EDID blob from a monitor
- Escalation via the config file, the log file path, or the single-instance mechanism
- Anything that lets another local process abuse the overlay or the low-level keyboard
  hook (`intercept_brightness_keys`) beyond what those features already do by design

Out of scope: issues that require an attacker who already runs code as the same user with
the same privileges (they can already do everything this tool can), crashes without a
security consequence (report those as normal bugs — gladly), and the SmartScreen/Defender
warnings discussed in the README.

## Regulatory status

This is free, non-commercial open-source software published by a private individual. It is
not made available on the market in the course of a commercial activity and therefore falls
outside the scope of Regulation (EU) 2024/2847 (Cyber Resilience Act) — see its Recital 18
and Art. 3(22); the "open-source software steward" role (Art. 3(14)) applies to legal
persons only. No SBOM, conformity assessment, or vulnerability-reporting commitment is
offered. If you integrate this software into a commercial product, CRA manufacturer
obligations attach to **you**, not to me.
