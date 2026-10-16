use entmoot_contracts::package_api::LockedPackageInstance;
use entmoot_contracts::{blake3_hex, ContentDigest};

#[test]
fn instance_identity_preserves_lock_entry_placement_version_and_case() {
    for purl in [
        "pkg:cargo/api@1.0.0",
        "pkg:pypi/api@1.0.0",
        "pkg:npm/api@1.0.0",
        "pkg:golang/example.org/Api@v1.0.0",
        "pkg:maven/org.example/api@1.0.0",
        "pkg:nuget/Api@1.0.0",
        "pkg:conan/api@1.0.0",
    ] {
        let mut instance = LockedPackageInstance {
            instance_id: String::new(),
            purl: purl.into(),
            lock_path: "locks/native.lock".into(),
            lock_digest: ContentDigest::parse(blake3_hex(b"retained lock bytes")).unwrap(),
            entry: "/packages/first".into(),
            placement: "deps/first".into(),
        };
        instance.instance_id = instance.derive_id();
        instance.validate().unwrap();
        let mut duplicate = instance.clone();
        duplicate.entry = "/packages/second".into();
        assert_ne!(instance.derive_id(), duplicate.derive_id());
        duplicate = instance.clone();
        duplicate.placement = "deps/nested/first".into();
        assert_ne!(instance.derive_id(), duplicate.derive_id());
        duplicate = instance.clone();
        duplicate.lock_digest = ContentDigest::parse(blake3_hex(b"changed lock")).unwrap();
        assert!(duplicate.validate().is_err());
        duplicate = instance.clone();
        duplicate.lock_path = "../outside.lock".into();
        duplicate.instance_id = duplicate.derive_id();
        assert!(duplicate.validate().is_err());
        duplicate = instance.clone();
        duplicate.purl = purl.split('@').next().unwrap().into();
        duplicate.instance_id = duplicate.derive_id();
        assert!(duplicate.validate().is_err());
    }
}
