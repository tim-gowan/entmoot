//! Roslyn sidecar deep checks. Product Open/Closed smoke rows live in
//! `expectations_smoke.json` / `dispatch_expectations` (`qualify_smoke`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "support/callsites_expect.rs"]
mod callsites_expect;

use callsites_expect::qualify_smoke;
use entmoot_contracts::BindingFamily;
use entmoot_engine::native_resolver::resolve_sidecar_family;

#[test]
fn csharp_smoke_product_claims() {
    let _adapter = resolve_sidecar_family("roslyn")
        .expect("catalog adapter required; build with scripts/build_resolver_adapters.py");
    std::env::set_var("ENTMOOT_ROSLYN_ADAPTER", &_adapter);
    qualify_smoke(
        "csharp",
        BindingFamily::Roslyn,
        Some("ENTMOOT_ROSLYN_ADAPTER"),
    );
}
