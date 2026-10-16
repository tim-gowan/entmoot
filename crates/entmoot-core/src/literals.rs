//! Shared integer-literal normalization for language normalizers.

/// Options controlling integer-literal base detection.
pub struct IntegerNormalizeOptions {
    /// When true, a leading-zero digit sequence (e.g. `077`) is treated as octal.
    /// JS/TS should pass false; Python/Java/Go/C#/Rust pass true.
    pub leading_zero_octal: bool,
}

/// Normalize an integer literal to a canonical decimal string when parseable.
pub fn normalize_integer_literal_text(literal_text: &str, options: IntegerNormalizeOptions) -> String {
    let Some((sign, digits, base)) = prepare_integer_literal(literal_text, options) else {
        return literal_text.to_string();
    };
    match i128::from_str_radix(&digits, base) {
        Ok(value) => (sign * value).to_string(),
        Err(_) => literal_text.to_string(),
    }
}

fn prepare_integer_literal(literal_text: &str, options: IntegerNormalizeOptions) -> Option<(i128, String, u32)> {
    let text = sanitize_integer_literal(literal_text)?;
    let (sign, text) = extract_integer_sign(&text);
    if text.is_empty() {
        return None;
    }
    let text = strip_integer_suffix(&text);
    if text.is_empty() {
        return None;
    }
    let (digits, base) = detect_integer_base(&text, options)?;
    if digits.is_empty() {
        return None;
    }
    Some((sign, digits, base))
}

fn sanitize_integer_literal(literal_text: &str) -> Option<String> {
    if literal_text.is_empty() {
        return None;
    }
    let text = literal_text.trim().replace('_', "");
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn extract_integer_sign(text: &str) -> (i128, String) {
    let sign = if text.starts_with('-') { -1 } else { 1 };
    let text = text.trim_start_matches(['+', '-']);
    (sign, text.to_string())
}

fn strip_integer_suffix(text: &str) -> String {
    if let Some(last) = text.chars().last() {
        if matches!(last, 'l' | 'L' | 'u' | 'U') {
            return text[..text.len() - 1].to_string();
        }
    }
    text.to_string()
}

fn detect_integer_base(text: &str, options: IntegerNormalizeOptions) -> Option<(String, u32)> {
    let lowered = text.to_lowercase();
    for (prefix, base) in [("0x", 16), ("0b", 2), ("0o", 8)] {
        if lowered.starts_with(prefix) {
            return Some((text[prefix.len()..].to_string(), base));
        }
    }
    if options.leading_zero_octal && text.len() > 1 && text.starts_with('0') && text.chars().all(|c| c.is_ascii_digit())
    {
        return Some((text[1..].to_string(), 8));
    }
    Some((text.to_string(), 10))
}
