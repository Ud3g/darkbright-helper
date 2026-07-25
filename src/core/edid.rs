//! EDID parsing for monitor identification.
//!
//! Extracts the manufacturer ID and the display-name / serial-number
//! descriptors from a raw EDID block, producing the [`MonitorId`] that
//! identifies a monitor across sessions, cable swaps, and OS handle churn.
//! Pure byte parsing — reading the EDID bytes from the OS stays in the
//! platform layer.

use crate::core::state::MonitorId;

/// Parses basic information from EDID binary data.
///
/// Returns `None` if the data is shorter than the 128-byte EDID base block.
/// Content is otherwise taken as-is: a block without string descriptors
/// falls back to a `"Generic Monitor"` model name and no serial.
#[must_use]
pub fn parse_edid(edid: &[u8]) -> Option<MonitorId> {
    if edid.len() < 128 {
        return None;
    }

    // Manufacturer ID (bytes 8-9)
    // Encoded as 5 bits per character (A=1, Z=26)
    let mfg_id = u16::from_be_bytes([edid[8], edid[9]]);
    let char1 = ((mfg_id >> 10) & 0x1F) as u8 + b'A' - 1;
    let char2 = ((mfg_id >> 5) & 0x1F) as u8 + b'A' - 1;
    let char3 = (mfg_id & 0x1F) as u8 + b'A' - 1;
    let manufacturer = String::from_utf8_lossy(&[char1, char2, char3]).to_string();

    // Model Name and Serial Number from Descriptors (bytes 54-125)
    let mut model_name = String::new();
    let mut serial_number = None;

    for i in 0..4 {
        let offset = 54 + i * 18;
        if offset + 18 > edid.len() {
            break;
        }
        let desc = &edid[offset..offset + 18];

        // Check for string descriptors (Flag: 00 00 00 xx 00)
        if desc[0] == 0 && desc[1] == 0 && desc[2] == 0 && desc[4] == 0 {
            let tag = desc[3];
            if tag == 0xFC {
                // Model Name
                model_name = parse_descriptor_string(&desc[5..]);
            } else if tag == 0xFF {
                // Serial Number
                serial_number = Some(parse_descriptor_string(&desc[5..]));
            }
        }
    }

    if model_name.is_empty() {
        model_name = "Generic Monitor".to_string();
    }

    Some(MonitorId::new(manufacturer, model_name, serial_number))
}

/// Helper to parse a string from an EDID descriptor block.
/// Strings are terminated by 0x0A (newline) or end of block.
fn parse_descriptor_string(bytes: &[u8]) -> String {
    let len = bytes.iter().position(|&b| b == 0x0A).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..len]).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 128-byte base block with a valid header and manufacturer "DEL"
    /// (5 bits per letter, A=1: D=4, E=5, L=12 → 0x10AC big-endian).
    fn base_block() -> Vec<u8> {
        let mut edid = vec![0u8; 128];
        edid[..8].copy_from_slice(&[0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00]);
        edid[8] = 0x10;
        edid[9] = 0xAC;
        edid
    }

    /// Writes an 18-byte display descriptor (tag 0xFC = model name, 0xFF =
    /// serial) into descriptor slot 0-3; the 13-byte text field is
    /// 0x0A-terminated and space-padded, as the EDID spec prescribes.
    fn put_descriptor(edid: &mut [u8], slot: usize, tag: u8, text: &str) {
        let offset = 54 + slot * 18;
        let desc = &mut edid[offset..offset + 18];
        desc[..5].copy_from_slice(&[0, 0, 0, tag, 0]);
        let mut field = [b' '; 13];
        field[..text.len()].copy_from_slice(text.as_bytes());
        if text.len() < 13 {
            field[text.len()] = 0x0A;
        }
        desc[5..].copy_from_slice(&field);
    }

    #[test]
    fn parses_manufacturer_model_and_serial() {
        let mut edid = base_block();
        put_descriptor(&mut edid, 0, 0xFC, "P2419H");
        put_descriptor(&mut edid, 1, 0xFF, "ABC123");

        assert_eq!(
            parse_edid(&edid),
            Some(MonitorId::new("DEL", "P2419H", Some("ABC123".to_string())))
        );
    }

    #[test]
    fn rejects_anything_shorter_than_the_base_block() {
        assert_eq!(parse_edid(&[]), None);
        assert_eq!(parse_edid(&[0u8; 127]), None);
    }

    #[test]
    fn missing_descriptors_fall_back_to_generic_name_and_no_serial() {
        assert_eq!(
            parse_edid(&base_block()),
            Some(MonitorId::new("DEL", "Generic Monitor", None))
        );
    }

    #[test]
    fn descriptor_text_is_cut_at_newline_and_trimmed() {
        let mut edid = base_block();
        put_descriptor(&mut edid, 0, 0xFC, " Spaced");
        assert_eq!(parse_edid(&edid).unwrap().model_name, "Spaced");
    }

    #[test]
    fn thirteen_char_name_without_terminator_is_used_in_full() {
        let mut edid = base_block();
        put_descriptor(&mut edid, 0, 0xFC, "ABCDEFGHIJKLM");
        assert_eq!(parse_edid(&edid).unwrap().model_name, "ABCDEFGHIJKLM");
    }

    #[test]
    fn zeroed_manufacturer_decodes_to_at_signs_not_rejection() {
        // The 5-bit PnP letters are defined for 1-26 (A-Z); a zeroed field
        // decodes to '@' rather than being rejected. Accepted quirk: the
        // value only ever feeds display names and identity strings.
        assert_eq!(
            parse_edid(&[0u8; 128]),
            Some(MonitorId::new("@@@", "Generic Monitor", None))
        );
    }

    #[test]
    fn bytes_after_the_base_block_are_ignored() {
        let mut edid = base_block();
        put_descriptor(&mut edid, 0, 0xFC, "Base Name");
        edid.extend_from_slice(&[0xAB; 128]); // extension block garbage
        assert_eq!(parse_edid(&edid).unwrap().model_name, "Base Name");
    }

    #[test]
    fn non_utf8_descriptor_bytes_do_not_panic_or_reject() {
        let mut edid = base_block();
        edid[54..59].copy_from_slice(&[0, 0, 0, 0xFC, 0]);
        for b in &mut edid[59..72] {
            *b = 0xEE;
        }
        let id = parse_edid(&edid).expect("invalid text must not reject the EDID");
        assert!(!id.model_name.is_empty());
    }
}
