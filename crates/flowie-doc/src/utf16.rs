//! Offsets in UTF-16 code units, as UIKit, AppKit's input methods and the browser count them,
//! against the byte offsets the outline keeps (wiki: iOS-Cross-Platform §3.3). Shared by the
//! desktop's text input and the native apps.

/// The UTF-16 offset of byte offset `byte` in `text` (past the end: the end; inside a
/// character: its start).
pub fn to_utf16(text: &str, byte: usize) -> usize {
    let mut byte = byte.min(text.len());
    while !text.is_char_boundary(byte) {
        byte -= 1;
    }
    text[..byte].encode_utf16().count()
}

/// The byte offset of UTF-16 offset `units` in `text` (past the end: the end; inside a
/// character, between the halves of a surrogate pair: its end).
pub fn from_utf16(text: &str, units: usize) -> usize {
    let mut utf16 = 0;
    for (i, ch) in text.char_indices() {
        if utf16 >= units {
            return i;
        }
        utf16 += ch.len_utf16();
    }
    text.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_round_trip_through_cjk_and_emoji() {
        // a: 1 byte, 1 unit; 日: 3 bytes, 1 unit; 😀: 4 bytes, 2 units.
        let text = "a日😀b";
        for (byte, units) in [(0, 0), (1, 1), (4, 2), (8, 4), (9, 5)] {
            assert_eq!(to_utf16(text, byte), units);
            assert_eq!(from_utf16(text, units), byte);
        }
        assert_eq!(to_utf16(text, 2), 1, "inside 日: its start");
        assert_eq!(from_utf16(text, 3), 8, "between the halves of 😀: its end");
        assert_eq!((to_utf16(text, 99), from_utf16(text, 99)), (5, 9), "past the end");
    }
}
