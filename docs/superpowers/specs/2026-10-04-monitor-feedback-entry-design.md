# Design: A tray entry that opens a prefilled hardware report

_Date: 2026-10-04_

## Problem

The tool is listed on winget and AlternativeTo, and it has received no user feedback at all. Reach
is the main cause, but the path back is also missing: someone who uses the tool has to find the
repository on their own, and once there neither the bug form (a satisfied user has no bug) nor an
empty "General" category invites a report.

Reports on working hardware are wanted as much as reports on broken hardware. DDC/CI behaves
differently per manufacturer, and a spread of monitor models is the reason the tool was published.

## Goal

One click in the tray menu opens a GitHub page in the browser with everything the app already knows
filled in. The user ticks boxes, optionally adds text, and submits.

Success is a handful of hardware reports, positive ones included, and a page that later promotion
can point at.

## Scope

**In scope:**

- A new tray menu entry, localized in all 15 interface languages.
- A fresh monitor enumeration when the entry is clicked, and a report built from that one pass.
- A count of displays the app could not identify.
- A pure, host-testable function that turns the report into a GitHub link.
- A new Discussions category and the repository files that route hardware reports to it.
- Documentation, including a consistency review of the existing documents.

**Out of scope (non-goals):**

- **A link in the settings window.** The tray is the surface every user knows. A third footer link
  would have to pass the layout tests in 15 languages at four DPIs; it can follow later without
  changing anything designed here.
- **A list of verified monitors in the README.** The reports are meant to feed one eventually.
- **A GitHub discussion form.** Forms are static and cannot hold one block per monitor.
- **Naming unidentified displays.** The app knows how many there are, not what they are.
- **Identical monitors without serial numbers, and clone mode.** Both already collapse to one entry
  in the app's model; this design inherits that and does not change it.

## Hard rules

- **No serial number in the link, ever.** The report type has no field that could carry one.
- **No paths, no user name, no configuration content in the link.**
- **The app sends nothing.** It opens a link. Submission happens in the browser, visibly and by the
  user's choice. The README's statement that the tool performs no network I/O stays true.

## Checks made before designing

| Question | Result |
|---|---|
| Does `discussions/new?category=…&title=…&body=…` prefill a category without a form? | **Yes.** Checked by hand against the "General" category: category, title and body arrived, including `%`, `[ ]` and line breaks |
| What does an unknown category slug do? | **No 404.** GitHub shows a banner and the category chooser at `/discussions/new/choose`. The address carries no query any more, so title and body are presumably lost |
| Can individual discussion-form fields be prefilled by URL? | **Not documented.** GitHub documents it for issue forms only. Not tested, because the design does not use a form |

The second result bounds the cost of the permanent contract below: a renamed category degrades old
versions to an empty editor, it does not break them.

## Design

### The GitHub side

A new Discussions category **"Hardware reports"**, slug `hardware-reports`, format "Open-ended
discussion", with no form attached. Suggested description, in the style of the existing ones:
"Which monitors work, and how. One post per setup." The description of "General" loses its mention
of hardware reports and becomes "Ideas, everything else."

Categories are created by hand in the web interface. The category has to exist before a release
carrying the link ships.

In the repository:

- `.github/ISSUE_TEMPLATE/config.yml`: the "Which monitors work?" contact link points at the new
  category and names the tray entry as the easiest way in.
- `SUPPORT.md`: the sentence on hardware reports points at the new category.
- `README.md`: a short pointer to the tray entry.

**The slug is a permanent contract.** Every shipped version has it compiled in.

The first report is the maintainer's own, produced with the new tray entry. It is the end-to-end
acceptance test, and it keeps the category from being empty when someone is pointed at it.

### What a report looks like

The text is English, like the rest of the GitHub side. Example for two monitors and one
unidentified display:

Title: `DEL U2722D + GSM LG ULTRAFINE`

```markdown
<!-- Filled in by darkbright-helper. Put an x in the brackets that
     apply ([x]), add anything you like, then press "Start discussion". -->

darkbright-helper 0.13.0 · Windows build 26200

### DEL U2722D
The app has read this monitor's brightness: yes
Connection (HDMI / DisplayPort / USB-C / dock):

Changing brightness with the hotkeys (1–100 %):
- [ ] works
- [ ] does not work

### GSM LG ULTRAFINE
The app has read this monitor's brightness: no
Connection (HDMI / DisplayPort / USB-C / dock):

Changing brightness with the hotkeys (1–100 %):
- [ ] works
- [ ] does not work

### 1 more display Windows reports that the app could not identify
It may be a virtual display. Model, if you know it:

### Dimming below 0 % (dark overlay)
- [ ] works
- [ ] does not work

### Anything else?
```

Decisions in this text:

- **Two boxes per question.** GitHub renders a task box only at the start of a list line, never
  side by side or inside a table. Two boxes make "does not work" an explicit answer, and both empty
  means "not tried".
- **What the app knows is a statement, not a box.** Per monitor the app knows one thing for
  certain: whether a brightness read has succeeded in this session (`brightness_known`). It keeps
  no per-monitor record of write success, so whether the hotkeys work is the user's answer.
- **"Has read" means at least once in this session**, not in the latest pass. A monitor in standby
  during the pass must not be reported as unreadable.
- **The overlay is asked about once.** It is a window and does not depend on the monitor model.
- **The connection type is asked.** The app does not know it.
- **The unidentified section appears only when the count is above zero**, with singular and plural.
- **No identified monitor at all** produces a report that says so, with the title "Hardware
  report". That is still a useful report.

### One enumeration pass, so that nothing is counted twice

The report lists identified monitors and counts unidentified displays. Three situations would put
one display into both:

1. **The wrong counting point.** `process_monitor` can fail at identification or later, when the
   DDC handle is opened. In the later case the monitor has already been reported as identified.
   Counting every failure would list it and count it.
2. **A transient identification failure.** A monitor identified earlier keeps its state for 90 s
   of absence (`PRUNE_ABSENCE_WINDOW`). If its EDID read fails once, it would still be listed from
   state and also be counted.
3. **A pass that identifies nothing.** The controller deliberately leaves all state untouched
   then. In a Remote Desktop session the report would list the physical monitors and count the
   virtual display beside them.

Summing counts across passes would be a fourth.

**Rule: a report is built from exactly one enumeration pass.** The list is the set of monitors
identified in that pass. The count is the number of displays in that same pass whose identification
failed, counted at that one failure site in the worker. Windows hands out each display once per
pass and each lands in exactly one of the two, so double counting is impossible by construction.

What remains:

- **The unit is a display as Windows sees it, not a device.** A virtual display without an EDID is
  counted. The wording therefore stays neutral and mentions the possibility.
- **Under-counting is possible.** Clone mode shows the app only the first device per image, and
  identical monitors without serial numbers share one identity. Both are existing limits.

### Flow

**DDC worker.** `handle_refresh_all` counts the displays for which `get_monitor_id` fails, and only
those. `DdcRefreshResult` gains a field `unidentified: usize`.

**Controller.**

- Keeps the topology of the last valid pass: the identified monitors and the unidentified count.
  A current-generation result replaces it. A result that carries no information (nothing
  identified and nothing counted) leaves it alone.
- A new message from the tray (working name `TrayShareMonitorFeedback`) starts a refresh and sets a
  flag that a report is waiting.
- The next current-generation refresh result fulfils the flag: the report is built from that pass
  and placed in an outbox. If that result carries no information, the report is built from the
  last valid topology instead.
- If the refresh cannot be started, or the watchdog aborts it after `REFRESH_TIMEOUT`, the flag is
  fulfilled from the last valid topology. If there has never been one, the report has no monitors.
- A second click while a report is waiting changes nothing but starts another refresh. One report
  results.

The flag waits for the *next valid result*, not for one particular generation. Another refresh can
begin in between (resume, a hotkey after inactivity) and would make the first generation stale; a
flag tied to it would never be fulfilled. Any later generation also began after the click, so it
serves the purpose equally.

A click always starts a new refresh, even when one is in flight: only then is the pass known to
have begun after the click.

**Main loop.** Once per iteration it takes the report out of the outbox, builds the link with the
app version and the Windows build, and opens it. This follows the existing pattern in which the
loop polls the controller for the language and the health warnings.

**Why an outbox.** The report comes into being inside the controller, later than the click, so the
existing interception of shell messages in `main.rs` cannot serve. An outbox keeps shell side
effects in `main.rs`, keeps the app version and the Windows build out of the controller, and needs
no new seam.

### Report data and link building

A new module in `core/` (working name `core/report.rs`) holds the report type and one pure function
from report, app version and optional Windows build to a link.

- **Report type:** per monitor a display name and whether its brightness has been read; plus the
  unidentified count. The display name is the one the tray menu shows (manufacturer code and model,
  with `#1`, `#2` for equal models). There is no field for a serial number.
- **Base address:** the `repository` value from `Cargo.toml`, plus
  `/discussions/new?category=hardware-reports&title=…&body=…`.
- **Percent-encoding:** a small function of its own, no new dependency. Everything except ASCII
  letters, digits and `- . _ ~` is encoded, as UTF-8.
- **Title:** the display names joined by " + ", in the order of the tray menu's rows. It is limited
  to 100 characters before encoding; names that do not fit are replaced by "+ N more".
- **Windows build:** from the registry read that `theme.rs` already performs. If it is unavailable
  the build is left out.

**Length budget.** The whole link is limited to 2000 characters. The figure rests on an unverified
recollection that opening a link through the Windows shell can fail near 2080 characters; the
manual pass checks it with a deliberately long link, and the budget is one constant. Measured on
the agreed wording, it holds four monitors with a full block. Beyond that the text is shortened
by whole blocks, never inside one: the monitors first in tray-menu order keep their full block,
the surplus ones appear as a single line "Also connected: …", and if that does not fit either,
as "+ N more".

**Opening.** `main.rs` gets a sibling of `open_with_default_app` for links, sharing the same
`ShellExecuteW` core.

### Tray entry and languages

The entry sits directly below "Settings", before "Open Log Folder".

English: **"Share monitor feedback…"**. German: **"Monitor-Rückmeldung geben…"**.

Every language's text has to meet four criteria:

1. It names the hardware: the word for "monitor" is in it.
2. It invites good and bad news alike.
3. It carries no overtone of a complaint or a fault report.
4. It uses no verb that, in that language, suggests the app itself transmits something. That would
   seem to contradict "the app sends nothing".

The other 13 texts come from the translation chain used for the earlier language batches, in
reduced form: translator, blind back-translation, reviewer, with the four criteria as the review
standard. The result is added to this document as an amendment before the strings are merged: per
language the text, its blind back-translation and the verdict on each criterion.

`Strings` gains one field. The existing i18n tests apply unchanged: no field empty, and equality
with English only as a declared decision. The menu sizes itself, so there is no layout risk.

## Rejected alternatives

- **An issue form.** Prefilling is documented there, but positive reports would sit in the bug
  tracker, and a form cannot hold a block per monitor.
- **A discussion form.** Field prefilling is undocumented, and the same static-form limit applies.
- **A pinned collection thread.** Replies cannot be prefilled at all.
- **The existing "General" category.** Reports would mix with everything else, and there would be
  no page to point at.
- **One-box questions.** An empty box would mean both "does not work" and "not answered".
- **Building the report from current state without a refresh.** The state can be up to a refresh
  interval old, and it mixes passes, which is what makes double counting possible.
- **A new seam as an eighth type parameter of the controller.** It would run through every
  signature and test setup for one method.
- **The controller posting a message to its own channel.** It would need a sender into its own
  inbox, a seam in disguise.

## Verification

**Unit tests, link function:** special characters and non-ASCII in model names; a serial-bearing
identity never shows up in the link; singular and plural; no monitors; the budget holds for any
number of monitors, and shortening never cuts inside an encoded sequence.

**Unit tests, controller:** a click starts a refresh; the result fulfils the report; a stale result
does not; a later generation does; an abort and a failed start fulfil from the last topology; two
clicks give one report; a monitor known from earlier but missing from the latest pass is not
listed; a result without information does not replace the stored topology.

**Manual, hardware-dependent:**

- The click opens the browser with the right monitors, in the right category.
- Unplug a monitor, click: it is not listed.
- A deliberately long link opens (checks the length budget).
- The count in the worker, as far as available hardware allows.
- The maintainer's own first report, submitted.

## Documentation impact

`docs/architecture.md` is the source of truth for behavior and gets thorough treatment:

- A description of the flow and of the one-pass rule with its reason.
- The statement on network freedom: the app opens a link and sends nothing.
- The refresh strategy gains a trigger; the passage on enumerated versus readable monitors gains
  the count; the tray section gains the entry and its message.
- A new manual procedure under "Integration Testing".

`README.md` (pointer to the entry; the privacy section says what the link contains) and
`SUPPORT.md` are updated. The entries in `CHANGELOG.md` and in the module map of `CLAUDE.md` are
kept brief.

**Consistency review.** After the documentation is written, a separate reviewer with no prior
context reads `docs/architecture.md`, `README.md`, `SUPPORT.md` and `CLAUDE.md` against the change
and reports every statement the change has made false or contradictory. All findings are resolved
before the pull request is opened.

## Risks

- **The length figure is an assumption.** Mitigated by the manual check and a single constant.
- **A GitHub account is required to submit.** Accepted; there is no other channel that keeps the
  app off the network.
- **GitHub may change its query parameters.** They are not in the official documentation for
  discussions. The app would then open an empty editor.
- **The wait before the browser opens.** Usually the duration of one refresh, at most
  `REFRESH_TIMEOUT` (5 s), with no visible progress.
- **Translation quality.** The texts are machine-translated and machine-reviewed, like the rest of
  the interface.
