//! Fences for local materializations; producer evidence retains its own identity.
use entmoot_contracts::{ContentDigest, ImplementationFingerprint};
use std::sync::OnceLock;

pub fn implementation_fingerprint() -> ImplementationFingerprint {
    static DERIVATION: OnceLock<ContentDigest> = OnceLock::new();
    ImplementationFingerprint {
        contracts: entmoot_contracts::contract_fingerprint(),
        derivation: DERIVATION
            .get_or_init(|| {
                let mut hash = blake3::Hasher::new();
                hash.update(b"entmoot.derivation_implementation.v1\0");
                for input in [
                    entmoot_contracts::PACKAGE_FINGERPRINT,
                    entmoot_core::PACKAGE_FINGERPRINT,
                    env!("ENTMOOT_PACKAGE_FINGERPRINT"),
                ] {
                    hash.update(input.as_bytes());
                }
                ContentDigest::from_trusted(format!("blake3:{}", hash.finalize().to_hex()))
            })
            .clone(),
    }
}
