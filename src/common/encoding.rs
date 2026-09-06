//! Encoding utilities for handling non-UTF-8 C source files.
//!
//! C source files may contain non-UTF-8 bytes in string/character literals
//! (e.g., EUC-JP, Latin-1 encoded files). Since Rust strings require valid
//! UTF-8, we encode non-UTF-8 bytes using Unicode Private Use Area (PUA)
//! code points, then decode them back to raw bytes in the lexer.
//!
//! Encoding scheme: byte 0x80+n → U+E080+n (PUA range U+E080..U+E0FF)

/// Base PUA code point for encoding non-UTF-8 bytes.
/// Byte 0x80 maps to U+E080, byte 0xFF maps to U+E0FF.
const PUA_BASE: u32 = 0xE080;

/// Convert raw bytes to a valid UTF-8 String.
///
/// If the bytes are valid UTF-8, returns them as-is.
/// Otherwise, processes byte-by-byte: valid UTF-8 sequences are preserved,
/// and invalid bytes 0x80-0xFF are encoded as PUA code points U+E080-U+E0FF.
///
/// A UTF-8 BOM (EF BB BF) at the start of the input is stripped, matching
/// the behavior of GCC and Clang.
pub fn bytes_to_string(bytes: Vec<u8>) -> String {
    // Strip UTF-8 BOM if present at the start of the file
    let bytes = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        bytes[3..].to_vec()
    } else {
        bytes
    };
    match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(e) => encode_non_utf8(e.into_bytes()),
    }
}

/// Encode a byte slice that contains non-UTF-8 data into a valid String
/// using PUA encoding for non-ASCII bytes that aren't part of valid UTF-8.
fn encode_non_utf8(bytes: Vec<u8>) -> String {
    let mut result = String::with_capacity(bytes.len() * 2);
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b < 0x80 {
            // ASCII byte - pass through directly
            result.push(b as char);
            i += 1;
        } else {
            // Try to decode a valid UTF-8 multi-byte sequence
            let seq_len = utf8_sequence_length(b);
            if seq_len > 1 && i + seq_len <= bytes.len() {
                if let Ok(s) = std::str::from_utf8(&bytes[i..i + seq_len]) {
                    result.push_str(s);
                    i += seq_len;
                    continue;
                }
            }
            // Not a valid UTF-8 sequence - encode as PUA
            result.push(char::from_u32(PUA_BASE + (b - 0x80) as u32).unwrap());
            i += 1;
        }
    }
    result
}

/// Determine the expected length of a UTF-8 sequence from its first byte.
fn utf8_sequence_length(b: u8) -> usize {
    if b < 0xC0 {
        1
    }
    // ASCII or continuation byte (invalid as start)
    else if b < 0xE0 {
        2
    } else if b < 0xF0 {
        3
    } else {
        4
    }
}

/// Decode a byte from the lexer's input, converting PUA-encoded bytes back
/// to their original values. Returns the decoded byte and how many input
/// bytes were consumed.
///
/// If the input at the given position contains a PUA-encoded byte
/// (U+E080..U+E0FF, which is the 3-byte UTF-8 sequence EE 82 80..EE 83 BF),
/// returns the original byte 0x80-0xFF and consumes 3 input bytes.
/// Otherwise, returns the input byte as-is and consumes 1 byte.
pub fn decode_pua_byte(input: &[u8], pos: usize) -> (u8, usize) {
    if pos + 2 < input.len() && input[pos] == 0xEE {
        // PUA U+E080-U+E0FF is encoded in UTF-8 as:
        // U+E080 = EE 82 80
        // U+E0BF = EE 82 BF
        // U+E0C0 = EE 83 80
        // U+E0FF = EE 83 BF
        let b1 = input[pos + 1];
        let b2 = input[pos + 2];
        if b1 == 0x82 && (0x80..=0xBF).contains(&b2) {
            // U+E080..U+E0BF → original byte 0x80..0xBF
            let orig = b2; // 0x80 + (b2 - 0x80) = b2
            return (orig, 3);
        } else if b1 == 0x83 && (0x80..=0xBF).contains(&b2) {
            // U+E0C0..U+E0FF → original byte 0xC0..0xFF
            let orig = 0xC0 + (b2 - 0x80);
            return (orig, 3);
        }
    }
    (input[pos], 1)
}

/// Decode the lexer's narrow-string raw-byte carrier as UTF-8 text.
///
/// `Lexer::lex_string` intentionally represents each narrow-string byte by a
/// Rust `char` in the U+0000..U+00FF range.  That lets ordinary C string data
/// preserve `\\xNN` and non-UTF-8 input byte-for-byte through codegen.  Inline
/// assembly is different: its template is later appended to a UTF-8 assembly
/// `String`, so a valid UTF-8 byte sequence must be decoded before that append
/// rather than encoded a second time as Latin-1 characters.
///
/// `None` means the carrier contains a non-byte character or is not valid
/// UTF-8.  Callers that can only transport Rust strings retain the original
/// carrier in that case rather than silently replacing or dropping bytes.
pub fn narrow_string_byte_carrier_to_utf8(carrier: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(carrier.chars().count());
    for ch in carrier.chars() {
        if (ch as u32) > u8::MAX as u32 {
            return None;
        }
        bytes.push(ch as u8);
    }
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ascii_passthrough() {
        let bytes = b"hello world".to_vec();
        let result = bytes_to_string(bytes);
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_valid_utf8_passthrough() {
        let bytes = "こんにちは".as_bytes().to_vec();
        let result = bytes_to_string(bytes);
        assert_eq!(result, "こんにちは");
    }

    #[test]
    fn test_non_utf8_encoding() {
        // EUC-JP byte sequence \xa4\xa2 (hiragana "a")
        let bytes = vec![0xA4, 0xA2];
        let encoded = bytes_to_string(bytes);
        // Should be encoded as PUA characters
        assert!(encoded.is_char_boundary(0));
        assert_eq!(encoded.len(), 6); // Two 3-byte PUA chars

        // Decode back
        let input: Vec<u8> = encoded.bytes().collect();
        let (b0, len0) = decode_pua_byte(&input, 0);
        assert_eq!(b0, 0xA4);
        assert_eq!(len0, 3);
        let (b1, len1) = decode_pua_byte(&input, 3);
        assert_eq!(b1, 0xA2);
        assert_eq!(len1, 3);
    }

    #[test]
    fn test_mixed_ascii_and_non_utf8() {
        let bytes = vec![b'h', b'i', 0xA4, 0xA2, b'!'];
        let encoded = bytes_to_string(bytes);
        let input: Vec<u8> = encoded.bytes().collect();
        // 'h' 'i' PUA PUA '!'
        let (b, l) = decode_pua_byte(&input, 0);
        assert_eq!((b, l), (b'h', 1));
        let (b, l) = decode_pua_byte(&input, 1);
        assert_eq!((b, l), (b'i', 1));
        let (b, l) = decode_pua_byte(&input, 2);
        assert_eq!((b, l), (0xA4, 3));
        let (b, l) = decode_pua_byte(&input, 5);
        assert_eq!((b, l), (0xA2, 3));
        let (b, l) = decode_pua_byte(&input, 8);
        assert_eq!((b, l), (b'!', 1));
    }

    #[test]
    fn test_bom_stripping() {
        // UTF-8 BOM followed by ASCII content
        let bytes = vec![0xEF, 0xBB, 0xBF, b'#', b'i', b'n', b'c'];
        let result = bytes_to_string(bytes);
        assert_eq!(result, "#inc");

        // BOM-only file
        let bytes = vec![0xEF, 0xBB, 0xBF];
        let result = bytes_to_string(bytes);
        assert_eq!(result, "");

        // No BOM - should be unchanged
        let bytes = vec![b'#', b'i', b'n', b'c'];
        let result = bytes_to_string(bytes);
        assert_eq!(result, "#inc");
    }

    #[test]
    fn test_roundtrip_all_bytes() {
        // Test that all byte values 0x80-0xFF round-trip correctly
        for b in 0x80u8..=0xFF {
            let encoded = bytes_to_string(vec![b]);
            let input: Vec<u8> = encoded.bytes().collect();
            let (decoded, _) = decode_pua_byte(&input, 0);
            assert_eq!(decoded, b, "Byte 0x{:02X} failed round-trip", b);
        }
    }

    #[test]
    fn narrow_byte_carrier_decodes_valid_utf8_once() {
        let text = "# MS09 UTF-8: café € 🦀";
        let carrier: String = text.bytes().map(char::from).collect();
        assert_ne!(carrier, text, "the test must exercise the byte carrier");
        assert_eq!(
            narrow_string_byte_carrier_to_utf8(&carrier).as_deref(),
            Some(text)
        );
    }

    #[test]
    fn narrow_byte_carrier_refuses_non_utf8_or_non_byte_text() {
        let invalid: String = [0xe9u8].into_iter().map(char::from).collect();
        assert_eq!(narrow_string_byte_carrier_to_utf8(&invalid), None);
        assert_eq!(narrow_string_byte_carrier_to_utf8("€"), None);
    }
}
