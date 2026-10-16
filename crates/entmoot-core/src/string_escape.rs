//! Shared string-escape decoding used by Python and Go visitors.

/// Decode C-style / Python-style / Go-style unicode and common escapes in `input`.
/// Returns `None` when an escape sequence is malformed.
pub fn decode_unicode_escape_bytes(input: &[u8]) -> Option<String> {
    let mut result = String::new();
    let mut i = 0;
    while i < input.len() {
        if input[i] == b'\\' && i + 1 < input.len() {
            i += 1;
            match input[i] {
                b'n' => result.push('\n'),
                b't' => result.push('\t'),
                b'r' => result.push('\r'),
                b'\\' => result.push('\\'),
                b'\'' => result.push('\''),
                b'"' => result.push('"'),
                b'a' => result.push('\x07'),
                b'b' => result.push('\x08'),
                b'f' => result.push('\x0c'),
                b'v' => result.push('\x0b'),
                b'x' if i + 2 < input.len() => {
                    let hex = std::str::from_utf8(&input[i + 1..i + 3]).ok()?;
                    let value = u8::from_str_radix(hex, 16).ok()?;
                    result.push(char::from(value));
                    i += 2;
                }
                b'u' if i + 4 < input.len() => {
                    let hex = std::str::from_utf8(&input[i + 1..i + 5]).ok()?;
                    let value = u16::from_str_radix(hex, 16).ok()?;
                    if let Some(ch) = char::from_u32(value as u32) {
                        result.push(ch);
                    }
                    i += 4;
                }
                b'U' if i + 8 < input.len() => {
                    let hex = std::str::from_utf8(&input[i + 1..i + 9]).ok()?;
                    let value = u32::from_str_radix(hex, 16).ok()?;
                    if let Some(ch) = char::from_u32(value) {
                        result.push(ch);
                    }
                    i += 8;
                }
                b'0'..=b'7' => {
                    let mut octal = String::new();
                    octal.push(input[i] as char);
                    let mut j = i + 1;
                    while j < input.len() && j < i + 3 && (b'0'..=b'7').contains(&input[j]) {
                        octal.push(input[j] as char);
                        j += 1;
                    }
                    if let Ok(value) = u8::from_str_radix(&octal, 8) {
                        result.push(char::from(value));
                    }
                    i = j - 1;
                }
                other => result.push(char::from(other)),
            }
        } else {
            result.push(char::from(input[i]));
        }
        i += 1;
    }
    Some(result)
}
