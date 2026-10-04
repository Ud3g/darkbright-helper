//! The hardware report a user can post from the tray menu.
//!
//! The app sends nothing. It builds a link to the repository's "Hardware
//! reports" discussion category with title and text already filled in, and
//! the shell opens that link in the browser; whether anything is submitted is
//! the user's decision there. See `docs/architecture.md` §13, "Hardware
//! Report".

/// One identified monitor as a hardware report lists it.
///
/// Deliberately not a `MonitorId`: there is no field a serial number could
/// travel in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReportMonitor {
    /// Display name as the tray menu shows it (manufacturer code and model,
    /// `#N` for equal models).
    pub(crate) name: String,
    /// Whether a brightness read has succeeded at least once in this session.
    pub(crate) brightness_read: bool,
}

/// What one enumeration pass saw, as far as a hardware report states it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HardwareReport {
    /// The monitors the pass identified, in tray-menu order.
    pub(crate) monitors: Vec<ReportMonitor>,
    /// Displays the same pass enumerated but could not identify.
    pub(crate) unidentified: usize,
}

/// Longest link handed to the shell. Opening a link through the shell is
/// assumed to become unreliable somewhat above this; see the manual
/// procedure in `docs/architecture.md`.
const MAX_URL_LEN: usize = 2000;

/// Longest discussion title, in characters, before encoding.
const MAX_TITLE_LEN: usize = 100;

/// Discussion category the link opens. Compiled into every release, so the
/// slug must never be renamed on the GitHub side.
const CATEGORY: &str = "hardware-reports";

/// Title when no monitor was identified, or none fits the title limit.
const GENERIC_TITLE: &str = "Hardware report";

/// Hidden in the rendered post; visible in the editor, where it tells the
/// user what to do.
const INTRO: &str = "<!-- Filled in by darkbright-helper. Put an x in the brackets that apply ([x]), add anything you like, then press \"Start discussion\". -->";

/// Stands in for the monitor blocks when the pass identified nothing.
const NO_MONITOR_SECTION: &str =
    "### No monitor identified\nThe app found no monitor it could identify.";

/// Asked once: the overlay is a window and does not depend on the model.
const OVERLAY_SECTION: &str =
    "### Dimming below 0 % (dark overlay)\n- [ ] works\n- [ ] does not work";

/// Free-text invitation that closes the report.
const CLOSING_SECTION: &str = "### Anything else?";

/// Builds the link that opens a prefilled hardware report.
///
/// The result never exceeds `MAX_URL_LEN` characters. When the full text does not
/// fit, the first monitors keep their block and the rest are listed in one
/// line, or only counted.
#[must_use]
pub fn discussion_url(
    report: &HardwareReport,
    app_version: &str,
    windows_build: Option<u32>,
) -> String {
    let total = report.monitors.len();
    let title = report_title(&report.monitors);

    // Richest shape first: every monitor with its block, then one block
    // fewer at a time with the dropped monitors listed in a line, then fewer
    // names. Whole blocks and whole names only, so nothing is ever cut
    // inside an encoded sequence.
    let shapes = (0..=total)
        .rev()
        .map(|full| (full, total - full))
        .chain((0..total).rev().map(|listed| (0, listed)));
    for (full, listed) in shapes {
        let url = assemble(
            &title,
            &report_body(report, app_version, windows_build, full, listed),
        );
        if url.len() <= MAX_URL_LEN {
            return url;
        }
    }
    // Not reachable with real EDID names; a title of unencodably long
    // names is the only thing left to give up.
    assemble(
        GENERIC_TITLE,
        &report_body(report, app_version, windows_build, 0, 0),
    )
}

/// Joins the repository address, the category and the two encoded values.
fn assemble(title: &str, body: &str) -> String {
    format!(
        "{}/discussions/new?category={CATEGORY}&title={}&body={}",
        env!("CARGO_PKG_REPOSITORY"),
        percent_encode(title),
        percent_encode(body)
    )
}

/// The discussion title: the monitor names, as many as fit.
fn report_title(monitors: &[ReportMonitor]) -> String {
    for shown in (1..=monitors.len()).rev() {
        let names: Vec<&str> = monitors
            .iter()
            .take(shown)
            .map(|monitor| monitor.name.as_str())
            .collect();
        let joined = names.join(" + ");
        let rest = monitors.len() - shown;
        let candidate = if rest == 0 {
            joined
        } else {
            format!("{joined} + {rest} more")
        };
        if candidate.chars().count() <= MAX_TITLE_LEN {
            return candidate;
        }
    }
    GENERIC_TITLE.to_string()
}

/// The report text: the first `full` monitors with a block each, the next
/// `listed` by name only, any after that counted.
fn report_body(
    report: &HardwareReport,
    app_version: &str,
    windows_build: Option<u32>,
    full: usize,
    listed: usize,
) -> String {
    let mut sections = vec![INTRO.to_string(), version_line(app_version, windows_build)];
    if report.monitors.is_empty() {
        sections.push(NO_MONITOR_SECTION.to_string());
    }
    sections.extend(report.monitors.iter().take(full).map(monitor_section));
    if let Some(line) = also_connected(&report.monitors, full, listed) {
        sections.push(line);
    }
    if let Some(section) = unidentified_section(report.unidentified) {
        sections.push(section);
    }
    sections.push(OVERLAY_SECTION.to_string());
    sections.push(CLOSING_SECTION.to_string());

    let mut text = sections.join("\n\n");
    // The cursor should land on a line of its own under the last heading.
    text.push('\n');
    text
}

/// The line naming the build that produced the report.
fn version_line(app_version: &str, windows_build: Option<u32>) -> String {
    match windows_build {
        Some(build) => format!("darkbright-helper {app_version} · Windows build {build}"),
        None => format!("darkbright-helper {app_version}"),
    }
}

/// One monitor's block: what the app knows, then what only the user knows.
fn monitor_section(monitor: &ReportMonitor) -> String {
    let read = if monitor.brightness_read { "yes" } else { "no" };
    format!(
        "### {}\n\
         The app has read this monitor's brightness: {read}\n\
         Connection (HDMI / DisplayPort / USB-C / dock):\n\
         \n\
         Changing brightness with the hotkeys (1–100 %):\n\
         - [ ] works\n\
         - [ ] does not work",
        monitor.name
    )
}

/// The line for monitors that did not get a block, or `None` if all did.
fn also_connected(monitors: &[ReportMonitor], full: usize, listed: usize) -> Option<String> {
    let surplus = monitors.len().saturating_sub(full);
    if surplus == 0 {
        return None;
    }
    let names: Vec<&str> = monitors
        .iter()
        .skip(full)
        .take(listed)
        .map(|monitor| monitor.name.as_str())
        .collect();
    let counted = surplus - names.len();
    Some(match (names.is_empty(), counted) {
        (true, _) => format!("Also connected: {counted} more"),
        (false, 0) => format!("Also connected: {}", names.join(", ")),
        (false, _) => format!("Also connected: {} + {counted} more", names.join(", ")),
    })
}

/// The section on displays the pass could not identify, or `None` for zero.
fn unidentified_section(count: usize) -> Option<String> {
    match count {
        0 => None,
        1 => Some(
            "### 1 more display Windows reports that the app could not identify\n\
             It may be a virtual display. Model, if you know it:"
                .to_string(),
        ),
        _ => Some(format!(
            "### {count} more displays Windows reports that the app could not identify\n\
             They may be virtual displays. Models, if you know them:"
        )),
    }
}

/// Percent-encodes `input` for use as a query value.
///
/// Everything except the unreserved characters of RFC 3986 is encoded, byte
/// by byte, from the UTF-8 form.
fn percent_encode(input: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(input.len() * 3);
    for byte in input.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push('%');
            out.push(char::from(HEX[usize::from(byte >> 4)]));
            out.push(char::from(HEX[usize::from(byte & 0x0F)]));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(name: &str, brightness_read: bool) -> ReportMonitor {
        ReportMonitor {
            name: name.to_string(),
            brightness_read,
        }
    }

    fn report(names: &[&str], unidentified: usize) -> HardwareReport {
        HardwareReport {
            monitors: names.iter().map(|name| monitor(name, true)).collect(),
            unidentified,
        }
    }

    /// Reverses [`percent_encode`]. Panics on a truncated or malformed
    /// sequence, which is what makes it a check on the encoder.
    fn decode(encoded: &str) -> String {
        let bytes = encoded.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] == b'%' {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3])
                    .expect("two characters follow a percent sign");
                out.push(u8::from_str_radix(hex, 16).expect("two hex digits"));
                index += 3;
            } else {
                out.push(bytes[index]);
                index += 1;
            }
        }
        String::from_utf8(out).expect("decoded text is UTF-8")
    }

    /// The decoded title and body of a link.
    fn title_and_body(url: &str) -> (String, String) {
        let query = url.split_once('?').expect("link has a query").1;
        let mut title = None;
        let mut body = None;
        for pair in query.split('&') {
            let (key, value) = pair.split_once('=').expect("key=value");
            match key {
                "title" => title = Some(decode(value)),
                "body" => body = Some(decode(value)),
                _ => {}
            }
        }
        (
            title.expect("title parameter"),
            body.expect("body parameter"),
        )
    }

    #[test]
    fn encodes_everything_outside_the_unreserved_set() {
        assert_eq!(percent_encode("Az09-._~"), "Az09-._~");
        assert_eq!(
            percent_encode("a b&c#d+e%f/ü–"),
            "a%20b%26c%23d%2Be%25f%2F%C3%BC%E2%80%93"
        );
        assert_eq!(percent_encode("- [ ]\n"), "-%20%5B%20%5D%0A");
    }

    #[test]
    fn link_opens_the_hardware_reports_category() {
        let url = discussion_url(&report(&["DEL U2722D"], 0), "0.13.0", Some(26200));
        assert!(
            url.starts_with(
                "https://github.com/Ud3g/darkbright-helper/discussions/new?category=hardware-reports&title="
            ),
            "{url}"
        );
    }

    #[test]
    fn two_monitors_and_one_unidentified_display_give_the_agreed_text() {
        let report = HardwareReport {
            monitors: vec![
                monitor("DEL U2722D", true),
                monitor("GSM LG ULTRAFINE", false),
            ],
            unidentified: 1,
        };
        let (title, body) = title_and_body(&discussion_url(&report, "0.13.0", Some(26200)));
        assert_eq!(title, "DEL U2722D + GSM LG ULTRAFINE");
        let expected = [
            INTRO,
            "",
            "darkbright-helper 0.13.0 · Windows build 26200",
            "",
            "### DEL U2722D",
            "The app has read this monitor's brightness: yes",
            "Connection (HDMI / DisplayPort / USB-C / dock):",
            "",
            "Changing brightness with the hotkeys (1–100 %):",
            "- [ ] works",
            "- [ ] does not work",
            "",
            "### GSM LG ULTRAFINE",
            "The app has read this monitor's brightness: no",
            "Connection (HDMI / DisplayPort / USB-C / dock):",
            "",
            "Changing brightness with the hotkeys (1–100 %):",
            "- [ ] works",
            "- [ ] does not work",
            "",
            "### 1 more display Windows reports that the app could not identify",
            "It may be a virtual display. Model, if you know it:",
            "",
            "### Dimming below 0 % (dark overlay)",
            "- [ ] works",
            "- [ ] does not work",
            "",
            "### Anything else?",
            "",
        ]
        .join("\n");
        assert_eq!(body, expected);
    }

    #[test]
    fn several_unidentified_displays_are_named_in_the_plural() {
        let (_, body) = title_and_body(&discussion_url(&report(&["DEL U2722D"], 3), "1.0.0", None));
        assert!(
            body.contains(
                "### 3 more displays Windows reports that the app could not identify\n\
                 They may be virtual displays. Models, if you know them:"
            ),
            "{body}"
        );
    }

    #[test]
    fn no_unidentified_section_when_every_display_was_identified() {
        let (_, body) = title_and_body(&discussion_url(&report(&["DEL U2722D"], 0), "1.0.0", None));
        assert!(!body.contains("could not identify"), "{body}");
    }

    #[test]
    fn no_monitors_gives_a_generic_title_and_says_so() {
        let (title, body) = title_and_body(&discussion_url(&report(&[], 1), "1.0.0", None));
        assert_eq!(title, "Hardware report");
        assert!(body.contains("### No monitor identified"), "{body}");
        assert!(
            body.contains("### 1 more display Windows reports"),
            "{body}"
        );
        assert!(body.contains("### Dimming below 0 %"), "{body}");
    }

    #[test]
    fn the_windows_build_is_left_out_when_unknown() {
        let (_, body) = title_and_body(&discussion_url(&report(&["DEL U2722D"], 0), "1.0.0", None));
        assert!(body.contains("\n\ndarkbright-helper 1.0.0\n\n"), "{body}");
        assert!(!body.contains("Windows build"), "{body}");
    }

    #[test]
    fn names_with_reserved_characters_survive_the_round_trip() {
        let name = "ACR K&R #1 + 100% ü";
        let (title, body) = title_and_body(&discussion_url(&report(&[name], 0), "1.0.0", None));
        assert_eq!(title, name);
        assert!(body.contains(&format!("### {name}\n")), "{body}");
    }

    #[test]
    fn three_monitors_all_keep_their_full_block() {
        let names = ["DEL U2722D #1", "DEL U2722D #2", "DEL U2722D #3"];
        let (_, body) = title_and_body(&discussion_url(&report(&names, 0), "0.13.0", Some(26200)));
        for name in names {
            assert!(
                body.contains(&format!("### {name}\n")),
                "{name} missing: {body}"
            );
        }
        assert!(!body.contains("Also connected"), "{body}");
    }

    #[test]
    fn surplus_monitors_are_named_in_one_line_and_the_first_keep_their_block() {
        let names: Vec<String> = (1..=8).map(|n| format!("DEL U2722D #{n}")).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let url = discussion_url(&report(&names, 0), "0.13.0", Some(26200));
        assert!(url.len() <= MAX_URL_LEN, "{} characters", url.len());
        let (_, body) = title_and_body(&url);
        assert!(body.contains("### DEL U2722D #1\n"), "{body}");
        assert!(!body.contains("### DEL U2722D #8\n"), "{body}");
        assert!(body.contains("Also connected: "), "{body}");
        assert!(body.contains("DEL U2722D #8"), "{body}");
    }

    #[test]
    fn a_long_title_ends_in_a_count() {
        let names: Vec<String> = (1..=8).map(|n| format!("DEL U2722D #{n}")).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let (title, _) = title_and_body(&discussion_url(&report(&names, 0), "0.13.0", None));
        assert!(title.chars().count() <= MAX_TITLE_LEN, "{title}");
        assert!(title.starts_with("DEL U2722D #1 + "), "{title}");
        assert!(title.ends_with(" more"), "{title}");
    }

    #[test]
    fn budget_holds_for_any_number_of_monitors() {
        for count in 0..=40 {
            let names: Vec<String> = (1..=count)
                .map(|n| format!("ÄÖÜ Überlanger Modellname & Co #{n}"))
                .collect();
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            let url = discussion_url(
                &report(&names, 2),
                "0.12.0+55.gc4687e5.dirty (dev)",
                Some(26200),
            );
            assert!(
                url.len() <= MAX_URL_LEN,
                "{count} monitors: {} characters",
                url.len()
            );
            let (_, body) = title_and_body(&url);
            assert!(body.contains("0.12.0+55.gc4687e5.dirty (dev)"), "{body}");
            assert!(body.ends_with("### Anything else?\n"), "{body}");
        }
    }
}
