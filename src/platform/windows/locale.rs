//! The OS's UI-language preference list, for the `"system"` language choice.

use windows::Win32::Globalization::{GetUserPreferredUILanguages, MUI_LANGUAGE_NAME};
use windows::core::PWSTR;

use crate::core::i18n::LanguageSource;

/// [`LanguageSource`] backed by `GetUserPreferredUILanguages`, the list the
/// Windows shell itself localises from (as opposed to the regional format,
/// which is a different setting).
pub struct WindowsLanguageSource;

/// Splits a double-NUL-terminated UTF-16 multi-string into its entries.
fn split_multi_string(buf: &[u16]) -> Vec<String> {
    buf.split(|&unit| unit == 0)
        .take_while(|entry| !entry.is_empty())
        .map(String::from_utf16_lossy)
        .collect()
}

impl LanguageSource for WindowsLanguageSource {
    fn preferred_languages(&self) -> Vec<String> {
        match read_preferred_ui_languages() {
            Ok(tags) => tags,
            Err(e) => {
                log::warn!(error:% = e; "Could not read the preferred UI languages; using English");
                Vec::new()
            }
        }
    }
}

/// Two calls, as documented: the first, with no buffer and a size of 0,
/// reports the size needed in UTF-16 units including both terminating NULs;
/// the second fills a buffer of that size.
fn read_preferred_ui_languages() -> windows::core::Result<Vec<String>> {
    let mut count: u32 = 0;
    let mut size: u32 = 0;
    unsafe {
        GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &raw mut count, None, &raw mut size)?;
    }
    let mut buf = vec![0u16; usize::try_from(size).unwrap_or(0)];
    // SAFETY: `buf` is exactly `size` units long, the size the first call
    // asked for, and `size` is passed back alongside it so the call cannot
    // write past the end; `PWSTR` points into `buf`, which outlives the call.
    unsafe {
        GetUserPreferredUILanguages(
            MUI_LANGUAGE_NAME,
            &raw mut count,
            Some(PWSTR(buf.as_mut_ptr())),
            &raw mut size,
        )?;
    }
    Ok(split_multi_string(&buf))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multi_string_splits_at_each_nul_and_stops_at_the_double_nul() {
        let buf: Vec<u16> = "de-DE\0en-US\0\0".encode_utf16().collect();
        assert_eq!(split_multi_string(&buf), vec!["de-DE", "en-US"]);
    }

    #[test]
    fn an_empty_multi_string_yields_nothing() {
        assert_eq!(split_multi_string(&[0, 0]), Vec::<String>::new());
        assert_eq!(split_multi_string(&[]), Vec::<String>::new());
    }

    #[test]
    fn the_os_reports_at_least_one_well_formed_language_tag() {
        // Pins MUI_LANGUAGE_NAME: the LANGID form would return hex such as
        // "0409", which the shape check below rejects. A hosted CI runner has
        // a UI language, so the list is expected to be non-empty there too.
        let tags = WindowsLanguageSource.preferred_languages();
        assert!(!tags.is_empty(), "no preferred UI language reported");
        for tag in &tags {
            let primary: String = tag.chars().take_while(|c| *c != '-').collect();
            assert!(
                (2..=3).contains(&primary.len())
                    && primary.chars().all(|c| c.is_ascii_alphabetic()),
                "{tag:?} does not start with a 2- or 3-letter primary subtag"
            );
        }
    }
}
