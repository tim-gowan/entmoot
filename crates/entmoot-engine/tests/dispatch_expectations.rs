//! Authored Open/Closed dispatch expectations driven by engine produce APIs.
//! Sidecar/Program adapters resolve from catalog defaults under
//! `.tmp/adapter-build/` when `ENTMOOT_*_ADAPTER` is unset. Floor-backed Closed
//! policies are listed in `tools/resolver-adapters/floors.toml`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "support/callsites_expect.rs"]
mod callsites_expect;

use callsites_expect::{qualify_construct, qualify_smoke};
use entmoot_contracts::BindingFamily;

// --- Rust (in-process; feature-gated) -------------------------------------------------

#[cfg(feature = "resolved-rust-analyzer")]
mod rust {
    use super::*;

    #[test]
    fn free_fn_closed() {
        qualify_construct("rust", "free_fn", BindingFamily::Rust, None);
    }

    #[test]
    fn inherent_closed() {
        qualify_construct("rust", "inherent", BindingFamily::Rust, None);
    }

    #[test]
    fn ufcs_closed() {
        qualify_construct("rust", "ufcs", BindingFamily::Rust, None);
    }

    #[test]
    fn trait_dyn_open() {
        qualify_construct("rust", "trait_dyn", BindingFamily::Rust, None);
    }

    #[test]
    fn callback_open() {
        qualify_construct("rust", "callback", BindingFamily::Rust, None);
    }

    #[test]
    fn smoke() {
        qualify_smoke("rust", BindingFamily::Rust, None);
    }
}

// --- Go -------------------------------------------------------------------------------

#[test]
fn go_free_fn_closed() {
    qualify_construct(
        "go",
        "free_fn",
        BindingFamily::Go,
        Some("ENTMOOT_GO_ADAPTER"),
    );
}

#[test]
fn go_concrete_method_closed() {
    qualify_construct(
        "go",
        "concrete_method",
        BindingFamily::Go,
        Some("ENTMOOT_GO_ADAPTER"),
    );
}

#[test]
fn go_iface_open() {
    qualify_construct("go", "iface", BindingFamily::Go, Some("ENTMOOT_GO_ADAPTER"));
}

#[test]
fn go_callback_open() {
    qualify_construct(
        "go",
        "callback",
        BindingFamily::Go,
        Some("ENTMOOT_GO_ADAPTER"),
    );
}

#[test]
fn go_smoke() {
    qualify_smoke("go", BindingFamily::Go, Some("ENTMOOT_GO_ADAPTER"));
}

// --- Java -----------------------------------------------------------------------------

#[test]
fn java_static_closed() {
    qualify_construct(
        "java",
        "static_call",
        BindingFamily::Java,
        Some("ENTMOOT_JAVA_ADAPTER"),
    );
}

#[test]
fn java_private_closed() {
    qualify_construct(
        "java",
        "private_call",
        BindingFamily::Java,
        Some("ENTMOOT_JAVA_ADAPTER"),
    );
}

#[test]
fn java_final_closed() {
    qualify_construct(
        "java",
        "final_method",
        BindingFamily::Java,
        Some("ENTMOOT_JAVA_ADAPTER"),
    );
}

#[test]
fn java_super_closed() {
    qualify_construct(
        "java",
        "super_call",
        BindingFamily::Java,
        Some("ENTMOOT_JAVA_ADAPTER"),
    );
}

#[test]
fn java_default_iface_open() {
    qualify_construct(
        "java",
        "default_iface",
        BindingFamily::Java,
        Some("ENTMOOT_JAVA_ADAPTER"),
    );
}

#[test]
fn java_open_virtual() {
    qualify_construct(
        "java",
        "open_virtual",
        BindingFamily::Java,
        Some("ENTMOOT_JAVA_ADAPTER"),
    );
}

#[test]
fn java_smoke() {
    qualify_smoke("java", BindingFamily::Java, Some("ENTMOOT_JAVA_ADAPTER"));
}

// --- C# / Roslyn ----------------------------------------------------------------------

#[test]
fn csharp_static_closed() {
    qualify_construct(
        "csharp",
        "static_call",
        BindingFamily::Roslyn,
        Some("ENTMOOT_ROSLYN_ADAPTER"),
    );
}

#[test]
fn csharp_sealed_closed() {
    qualify_construct(
        "csharp",
        "sealed_method",
        BindingFamily::Roslyn,
        Some("ENTMOOT_ROSLYN_ADAPTER"),
    );
}

#[test]
fn csharp_base_closed() {
    qualify_construct(
        "csharp",
        "base_call",
        BindingFamily::Roslyn,
        Some("ENTMOOT_ROSLYN_ADAPTER"),
    );
}

#[test]
fn csharp_partial_closed() {
    qualify_construct(
        "csharp",
        "partial_method",
        BindingFamily::Roslyn,
        Some("ENTMOOT_ROSLYN_ADAPTER"),
    );
}

#[test]
fn csharp_expression_body_closed() {
    qualify_construct(
        "csharp",
        "expression_body",
        BindingFamily::Roslyn,
        Some("ENTMOOT_ROSLYN_ADAPTER"),
    );
}

#[test]
fn csharp_ordinary_instance_closed() {
    qualify_construct(
        "csharp",
        "ordinary_instance",
        BindingFamily::Roslyn,
        Some("ENTMOOT_ROSLYN_ADAPTER"),
    );
}

#[test]
fn csharp_open_virtual() {
    qualify_construct(
        "csharp",
        "open_virtual",
        BindingFamily::Roslyn,
        Some("ENTMOOT_ROSLYN_ADAPTER"),
    );
}

#[test]
fn csharp_smoke() {
    qualify_smoke(
        "csharp",
        BindingFamily::Roslyn,
        Some("ENTMOOT_ROSLYN_ADAPTER"),
    );
}

// --- C family / Clang -----------------------------------------------------------------

#[test]
fn c_family_non_virtual_closed() {
    qualify_construct(
        "c_family",
        "non_virtual_c",
        BindingFamily::Clang,
        Some("ENTMOOT_CLANG_ADAPTER"),
    );
}

#[test]
fn c_family_virtual_cxx_open() {
    qualify_construct(
        "c_family",
        "virtual_cxx",
        BindingFamily::Clang,
        Some("ENTMOOT_CLANG_ADAPTER"),
    );
}

// --- JavaScript / Oxc -----------------------------------------------------------------

#[cfg(feature = "resolved-oxc")]
mod javascript {
    use super::*;

    #[test]
    fn identifier_closed() {
        qualify_construct("javascript", "identifier", BindingFamily::Oxc, None);
    }

    #[test]
    fn member_open() {
        qualify_construct("javascript", "member", BindingFamily::Oxc, None);
    }

    #[test]
    fn arrow_const_open() {
        qualify_construct("javascript", "arrow_const", BindingFamily::Oxc, None);
    }
}

// --- Python ---------------------------------------------------------------------------

#[test]
fn python_unique_fn_closed() {
    qualify_construct(
        "python",
        "unique_fn",
        BindingFamily::Python,
        Some("ENTMOOT_PYRIGHT_ADAPTER"),
    );
}

#[test]
fn python_unique_member_closed() {
    qualify_construct(
        "python",
        "unique_member",
        BindingFamily::Python,
        Some("ENTMOOT_PYRIGHT_ADAPTER"),
    );
}

#[test]
fn python_decorator_open() {
    qualify_construct(
        "python",
        "decorator_open",
        BindingFamily::Python,
        Some("ENTMOOT_PYRIGHT_ADAPTER"),
    );
}

// --- TypeScript Program ---------------------------------------------------------------

#[test]
fn typescript_unique_signature_closed() {
    qualify_construct(
        "typescript",
        "unique_signature",
        BindingFamily::Typescript,
        Some("ENTMOOT_TYPESCRIPT_ADAPTER"),
    );
}

#[test]
fn typescript_call_signature_open() {
    qualify_construct(
        "typescript",
        "call_signature_open",
        BindingFamily::Typescript,
        Some("ENTMOOT_TYPESCRIPT_ADAPTER"),
    );
}
