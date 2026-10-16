//! Structural and address hashing helpers (BLAKE3).

/// BLAKE3 hex fingerprint of canonical normalized IR with `blake3:` prefix.
pub fn hash_structural_ir(canonical: &str) -> String {
    format!("blake3:{}", blake3::hash(canonical.as_bytes()).to_hex())
}

/// BLAKE3 hex of a logic address envelope (no prefix).
pub fn hash_address_envelope(canonical: &str) -> String {
    blake3::hash(canonical.as_bytes()).to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structural_ir_has_prefix() {
        assert!(hash_structural_ir("x").starts_with("blake3:"));
    }

    #[test]
    fn address_id_no_prefix() {
        let id = hash_address_envelope("logic|-|-|-|-|-|-|-|-");
        assert!(!id.starts_with("blake3:"));
        assert_eq!(id.len(), 64);
    }
}
