//! The user-visible string table.
//!
//! Every string a user can read lives here, once per language, so the UI can
//! be switched between languages without hunting literals across the platform
//! layer. Log messages are deliberately absent: they stay English so that a
//! pasted log is readable by whoever is diagnosing it.
//!
//! [`Strings`] is a plain struct rather than a map or an external catalog
//! because that makes completeness a compile-time property. Adding a field
//! breaks every language's initializer until it is filled in, and removing one
//! breaks every reference. No catalog format offers that.
//!
//! The product name `darkbright-helper` is not in here — a product name is not
//! translated.

/// A language the user interface can be displayed in.
///
/// Only English exists today. The enum is here so that consumers already take
/// a language parameter and adding a second one touches no call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lang {
    /// English, the fallback for every unmatched locale.
    #[default]
    English,
}

impl Lang {
    /// Every language the app can display, in the order the picker shows them.
    pub const ALL: &'static [Lang] = &[Lang::English];

    /// The BCP-47 tag identifying this language, for locale matching and for
    /// the value stored in the config file.
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            Lang::English => "en",
        }
    }
}

/// Every user-visible string, for one language.
///
/// Fields are grouped by the surface they appear on. Keep the groups and their
/// order identical in every language table so the two can be diffed.
#[derive(Debug, Clone, Copy)]
pub struct Strings {
    // --- On-screen display ---
    /// Shown in the OSD's error row when a DDC write failed.
    pub osd_ddc_error: &'static str,
}

/// The English strings. Every other language is a translation of this table.
pub const ENGLISH: Strings = Strings {
    osd_ddc_error: "DDC Error - Adjustment failed",
};

/// The string table for `lang`.
#[must_use]
pub fn strings(lang: Lang) -> &'static Strings {
    match lang {
        Lang::English => &ENGLISH,
    }
}

#[cfg(test)]
mod tests {
    use super::{ENGLISH, Lang, strings};

    #[test]
    fn every_language_tag_is_unique_and_lowercase() {
        let mut seen = Vec::new();
        for lang in Lang::ALL {
            let tag = lang.tag();
            assert!(!tag.is_empty(), "{lang:?} has an empty tag");
            assert_eq!(tag, tag.to_lowercase(), "{lang:?} tag is not lowercase");
            assert!(!seen.contains(&tag), "duplicate tag {tag}");
            seen.push(tag);
        }
    }

    #[test]
    fn every_language_resolves_to_a_table_with_no_empty_fields() {
        for &lang in Lang::ALL {
            let s = strings(lang);
            assert!(
                !s.osd_ddc_error.is_empty(),
                "{lang:?} has an empty osd_ddc_error"
            );
        }
    }

    #[test]
    fn english_is_the_table_returned_for_english() {
        assert_eq!(strings(Lang::English).osd_ddc_error, ENGLISH.osd_ddc_error);
    }
}
