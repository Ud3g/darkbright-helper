//! The version string the app shows to users.
//!
//! The package version alone cannot answer "which build is this?": it stays
//! at the last released number for the whole development cycle that follows,
//! so a binary built from a work-in-progress tree reports the same version as
//! the release it succeeded. Bug reports become ambiguous exactly when the
//! tree has moved on.
//!
//! `build.rs` closes that gap by passing `git describe` output in through the
//! `DBH_GIT_DESCRIBE` compile-time variable. This module turns the two into
//! one display string, adding a suffix *only* when the build is not a clean
//! checkout of the released tag:
//!
//! | Build | String |
//! | --- | --- |
//! | exactly on tag `0.9.0`, clean tree | `0.9.0` |
//! | no git available (release archive) | `0.9.0` |
//! | 55 commits past the tag | `0.9.0+55.gc4687e5 (dev)` |
//! | on the tag, uncommitted changes | `0.9.0+dirty (dev)` |
//! | both | `0.9.0+55.gc4687e5.dirty (dev)` |
//!
//! The suffix is semver build metadata (everything after `+`), which is
//! ignored when versions are compared — hence dots as separators, where
//! `git describe` natively writes dashes. `(dev)` follows in plain words
//! because `+55.gc4687e5` means nothing to someone who does not read git.

use std::sync::OnceLock;

/// The full version string for display in the tray menu, the settings window
/// and the startup log — see the module docs for the exact forms.
///
/// Computed once and cached; every caller gets the same borrowed string.
#[must_use]
pub fn version_string() -> &'static str {
    static CACHE: OnceLock<String> = OnceLock::new();
    CACHE.get_or_init(|| format_version(env!("CARGO_PKG_VERSION"), option_env!("DBH_GIT_DESCRIBE")))
}

/// Combines the package version with raw `git describe` output.
///
/// `describe` is `None` when the build had no usable git information at all,
/// which is the normal case for a source archive.
fn format_version(pkg: &str, describe: Option<&str>) -> String {
    match build_metadata(pkg, describe) {
        Some(extra) => format!("{pkg}+{extra} (dev)"),
        None => pkg.to_owned(),
    }
}

/// The `+…` part, or `None` when this build is a clean checkout of the tag
/// that matches `pkg` (and so should show no suffix at all).
///
/// The tag prefix is stripped only when it *is* the package version. When the
/// two disagree — the window between bumping `Cargo.toml` and tagging that
/// commit — the whole describe output is kept instead, because its commit
/// count is measured from the older tag and would misread as a distance from
/// the package version.
fn build_metadata(pkg: &str, describe: Option<&str>) -> Option<String> {
    let raw = describe.map(str::trim).filter(|raw| !raw.is_empty())?;
    if raw == pkg {
        return None;
    }
    let rest = raw
        .strip_prefix(pkg)
        .and_then(|rest| rest.strip_prefix('-'))
        .unwrap_or(raw);
    if rest.is_empty() {
        return None;
    }
    Some(rest.replace('-', "."))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PKG: &str = "0.9.0";

    #[test]
    fn no_git_information_shows_the_bare_package_version() {
        assert_eq!(format_version(PKG, None), "0.9.0");
    }

    #[test]
    fn clean_checkout_of_the_tag_shows_the_bare_package_version() {
        assert_eq!(format_version(PKG, Some("0.9.0")), "0.9.0");
    }

    #[test]
    fn commits_past_the_tag_show_count_and_hash() {
        assert_eq!(
            format_version(PKG, Some("0.9.0-55-gc4687e5")),
            "0.9.0+55.gc4687e5 (dev)"
        );
    }

    #[test]
    fn dirty_tree_on_the_tag_shows_dirty() {
        assert_eq!(
            format_version(PKG, Some("0.9.0-dirty")),
            "0.9.0+dirty (dev)"
        );
    }

    #[test]
    fn dirty_tree_past_the_tag_shows_both() {
        assert_eq!(
            format_version(PKG, Some("0.9.0-55-gc4687e5-dirty")),
            "0.9.0+55.gc4687e5.dirty (dev)"
        );
    }

    #[test]
    fn describe_without_any_tag_shows_the_bare_hash() {
        // `git describe --always` in a clone whose tags were not fetched.
        assert_eq!(format_version(PKG, Some("c4687e5")), "0.9.0+c4687e5 (dev)");
    }

    #[test]
    fn tag_older_than_the_package_version_is_kept_whole() {
        // Between the release bump and its tag: "3 commits" counts from
        // 0.9.0, so dropping that tag would misattribute them to 0.10.0.
        assert_eq!(
            format_version("0.10.0", Some("0.9.0-3-gabc1234")),
            "0.10.0+0.9.0.3.gabc1234 (dev)"
        );
    }

    #[test]
    fn empty_or_whitespace_describe_output_is_treated_as_absent() {
        assert_eq!(format_version(PKG, Some("")), "0.9.0");
        assert_eq!(format_version(PKG, Some("   ")), "0.9.0");
    }

    #[test]
    fn trailing_newline_from_the_git_process_is_trimmed() {
        assert_eq!(
            format_version(PKG, Some("0.9.0-55-gc4687e5\n")),
            "0.9.0+55.gc4687e5 (dev)"
        );
    }

    #[test]
    fn the_real_build_reports_at_least_the_package_version() {
        let actual = version_string();
        assert!(
            actual.starts_with(env!("CARGO_PKG_VERSION")),
            "version string {actual:?} does not start with the package version"
        );
    }
}
