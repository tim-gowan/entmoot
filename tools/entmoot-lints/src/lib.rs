#![feature(rustc_private)]
#![warn(unused_extern_crates)]

extern crate rustc_hir;
extern crate rustc_lint;
extern crate rustc_session;
extern crate rustc_span;

use clippy_utils::diagnostics::span_lint;
use rustc_hir::def::Res;
use rustc_hir::def_id::LOCAL_CRATE;
use rustc_hir::{AmbigArg, HirId, Item, ItemKind, Path, QPath, Ty, TyKind, UsePath};
use rustc_lint::{LateContext, LateLintPass, LintStore};
use rustc_session::{declare_lint, declare_lint_pass, Session};
use rustc_span::Span;

dylint_linting::dylint_library!();

declare_lint! {
    /// ### What it does
    ///
    /// Forbids `sha2` / `sha256` crate paths. Identity digests must use BLAKE3.
    pub BANNED_IDENTITY_DIGEST,
    Deny,
    "identity digests must use BLAKE3; sha2/sha256 are forbidden"
}

declare_lint! {
    /// ### What it does
    ///
    /// Forbids `Mutex<Arc<[u8]>>` / `Mutex<Arc<Vec<u8>>>` Sync hacks.
    pub BANNED_SYNC_BYTE_MUTEX,
    Deny,
    "forbidden Sync byte-buffer Mutex pattern"
}

declare_lint! {
    /// ### What it does
    ///
    /// Restricts `anyhow` to `entmoot-cli` and `entmoot-mcp` surfaces.
    pub ANYHOW_SURFACE_ONLY,
    Deny,
    "anyhow is allowed only in entmoot-cli and entmoot-mcp"
}

declare_lint! {
    /// ### What it does
    ///
    /// Forbids vendor AST / DB imports inside `entmoot-contracts`.
    pub CONTRACTS_NO_VENDOR_SYNTAX,
    Deny,
    "entmoot-contracts must not import tree-sitter or rusqlite types"
}

declare_lint_pass!(EntmootPolicy => [
    BANNED_IDENTITY_DIGEST,
    BANNED_SYNC_BYTE_MUTEX,
    ANYHOW_SURFACE_ONLY,
    CONTRACTS_NO_VENDOR_SYNTAX
]);

#[unsafe(no_mangle)]
pub fn register_lints(_sess: &Session, lint_store: &mut LintStore) {
    lint_store.register_lints(&[
        BANNED_IDENTITY_DIGEST,
        BANNED_SYNC_BYTE_MUTEX,
        ANYHOW_SURFACE_ONLY,
        CONTRACTS_NO_VENDOR_SYNTAX,
    ]);
    lint_store.register_late_lint_pass(Box::new(|_| Box::new(EntmootPolicy)));
}

fn local_crate_name(cx: &LateContext<'_>) -> String {
    cx.tcx.crate_name(LOCAL_CRATE).as_str().replace('-', "_")
}

/// Surface crates may use `anyhow` (package libs and their binary crate names).
fn allows_anyhow(crate_name: &str) -> bool {
    // Package names plus [[bin]].name (hyphens → underscores).
    matches!(
        crate_name,
        "entmoot_cli" | "entmoot_mcp" | "entmoot" | "entmoot_producer"
    )
}

fn path_segments(path: &Path<'_>) -> Vec<String> {
    path.segments
        .iter()
        .map(|segment| segment.ident.as_str().to_owned())
        .collect()
}

fn use_path_segments(path: &UsePath<'_>) -> Vec<String> {
    path.segments
        .iter()
        .map(|segment| segment.ident.as_str().to_owned())
        .collect()
}

fn first_segment(segments: &[String]) -> Option<&str> {
    segments.first().map(String::as_str)
}

fn lint_import_roots(cx: &LateContext<'_>, span: Span, segments: &[String]) {
    let Some(root) = first_segment(segments) else {
        return;
    };
    let name = local_crate_name(cx);

    if matches!(root, "sha2" | "sha256") {
        span_lint(
            cx,
            BANNED_IDENTITY_DIGEST,
            span,
            "identity digests must use BLAKE3; do not import sha2/sha256",
        );
    }

    if root == "anyhow" && !allows_anyhow(&name) {
        span_lint(
            cx,
            ANYHOW_SURFACE_ONLY,
            span,
            "anyhow is reserved for entmoot-cli / entmoot-mcp; libraries use typed errors",
        );
    }

    if name == "entmoot_contracts"
        && (root == "rusqlite" || root.starts_with("tree_sitter"))
    {
        span_lint(
            cx,
            CONTRACTS_NO_VENDOR_SYNTAX,
            span,
            "entmoot-contracts must stay free of tree-sitter and rusqlite imports",
        );
    }
}

fn lint_resolved_crate(cx: &LateContext<'_>, span: Span, path: &Path<'_>) {
    let Res::Def(_, def_id) = path.res else {
        return;
    };
    if def_id.is_local() {
        return;
    }
    let crate_name = cx.tcx.crate_name(def_id.krate).to_string();
    let name = local_crate_name(cx);

    if matches!(crate_name.as_str(), "sha2" | "sha256") {
        span_lint(
            cx,
            BANNED_IDENTITY_DIGEST,
            span,
            "identity digests must use BLAKE3; do not use sha2/sha256",
        );
    }

    if crate_name == "anyhow" && !allows_anyhow(&name) {
        span_lint(
            cx,
            ANYHOW_SURFACE_ONLY,
            span,
            "anyhow is reserved for entmoot-cli / entmoot-mcp; libraries use typed errors",
        );
    }

    if name == "entmoot_contracts"
        && (crate_name == "rusqlite" || crate_name.starts_with("tree_sitter"))
    {
        span_lint(
            cx,
            CONTRACTS_NO_VENDOR_SYNTAX,
            span,
            "entmoot-contracts must stay free of tree-sitter and rusqlite imports",
        );
    }
}

fn is_u8_path(path: &Path<'_>) -> bool {
    path_segments(path).last().is_some_and(|last| last == "u8")
}

fn is_u8_slice_or_vec(ty: &Ty<'_>) -> bool {
    match ty.kind {
        TyKind::Slice(inner) => {
            matches!(inner.kind, TyKind::Path(QPath::Resolved(_, path)) if is_u8_path(path))
        }
        TyKind::Path(QPath::Resolved(_, path)) => {
            let segments = path_segments(path);
            segments.last().is_some_and(|last| last == "Vec")
                && path
                    .segments
                    .last()
                    .and_then(|segment| segment.args)
                    .is_some_and(|args| {
                        args.args.iter().any(|arg| match arg {
                            rustc_hir::GenericArg::Type(inner) => {
                                matches!(
                                    inner.as_unambig_ty().kind,
                                    TyKind::Path(QPath::Resolved(_, path)) if is_u8_path(path)
                                )
                            }
                            _ => false,
                        })
                    })
        }
        _ => false,
    }
}

fn is_arc_of_bytes(ty: &Ty<'_>) -> bool {
    let TyKind::Path(QPath::Resolved(_, path)) = ty.kind else {
        return false;
    };
    let segments = path_segments(path);
    if segments.last().is_none_or(|last| last != "Arc") {
        return false;
    }
    path.segments
        .last()
        .and_then(|segment| segment.args)
        .is_some_and(|args| {
            args.args.iter().any(|arg| match arg {
                rustc_hir::GenericArg::Type(inner) => is_u8_slice_or_vec(inner.as_unambig_ty()),
                _ => false,
            })
        })
}

fn is_banned_sync_byte_mutex(ty: &Ty<'_>) -> bool {
    let TyKind::Path(QPath::Resolved(_, path)) = ty.kind else {
        return false;
    };
    let segments = path_segments(path);
    if segments.last().is_none_or(|last| last != "Mutex") {
        return false;
    }
    path.segments
        .last()
        .and_then(|segment| segment.args)
        .is_some_and(|args| {
            args.args.iter().any(|arg| match arg {
                rustc_hir::GenericArg::Type(inner) => is_arc_of_bytes(inner.as_unambig_ty()),
                _ => false,
            })
        })
}

impl<'tcx> LateLintPass<'tcx> for EntmootPolicy {
    fn check_item(&mut self, cx: &LateContext<'tcx>, item: &'tcx Item<'tcx>) {
        if let ItemKind::Use(path, _) = item.kind {
            lint_import_roots(cx, item.span, &use_path_segments(path));
        }
    }

    fn check_path(&mut self, cx: &LateContext<'tcx>, path: &Path<'tcx>, _hir_id: HirId) {
        lint_resolved_crate(cx, path.span, path);
    }

    fn check_ty(&mut self, cx: &LateContext<'tcx>, ty: &'tcx Ty<'tcx, AmbigArg>) {
        if is_banned_sync_byte_mutex(ty.as_unambig_ty()) {
            span_lint(
                cx,
                BANNED_SYNC_BYTE_MUTEX,
                ty.span,
                "Mutex<Arc<[u8]>> / Mutex<Arc<Vec<u8>>> Sync hacks are forbidden",
            );
        }
    }
}
