# Support

## What to expect

This is a spare-time project by one person with a full-time job and a family. I will
generally look at problems, but **there is no response-time commitment of any kind** — an
issue may be answered quickly, sit for a very long time, or in the worst case never be
resolved. Long quiet periods are normal and do not mean the project is abandoned. Details:
[Support and cadence](README.md#support-and-cadence) in the README.

Security reports go through a separate, private channel — see [SECURITY.md](SECURITY.md),
not the issue tracker.

## Before opening an issue

1. Update to the **latest release** — only that one receives fixes.
2. Skim the README, especially
   [Brightness Key Limitations](README.md#brightness-key-limitations) (dedicated
   brightness keys not working is usually your keyboard's firmware, not a bug) and
   [Running an unsigned binary](README.md#running-an-unsigned-binary) (SmartScreen and
   Defender warnings are expected and documented).
3. Search existing issues, including closed ones.

**If you are not sure whether it is a bug, it belongs in
[Q&A](https://github.com/Ud3g/darkbright-helper/discussions/categories/q-a) rather than
here.** An issue says the tool deviates from documented behaviour, and the bug report asks
for a debug log to prove it. A question does not need either. Reports on how the tool
behaves on your particular monitors are welcome in
[General](https://github.com/Ud3g/darkbright-helper/discussions/categories/general) —
DDC/CI implementations vary wildly, so that information is genuinely useful to me.

## How to report a problem so I can actually investigate it

For almost anything beyond a typo, I need a **debug log** — without it, most hardware
problems are guesswork:

1. In `%APPDATA%\BrightnessControl\config.json`, set:
   ```json
   "logging": { "file_enabled": true, "file_level": "debug" }
   ```
2. Restart the tool and reproduce the problem.
3. Attach `%APPDATA%\BrightnessControl\darkbright.log` to the issue.

**Privacy note before you attach it:** at `debug` level the log contains your monitors'
serial numbers and absolute paths including your Windows user name. Skim it first and
redact what you are not comfortable posting publicly — an issue attachment is public.

Please also state your Windows version, your monitor model(s) and connection type
(HDMI/DP/USB-C), and whether the problem is on the DDC/CI path (1–100%) or the overlay
path (below 0%).

**A caveat on monitor-specific problems:** DDC/CI implementations vary wildly between
monitors. If I cannot reproduce your problem on my own hardware, there may be little I can
do, and some of these issues will be closed without a fix.
