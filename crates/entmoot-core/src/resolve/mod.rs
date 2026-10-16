//! Import-local call resolution over [`crate::FileMarkers`] / [`crate::FileIndexRecord`].
//!
//! Pure post-pass: no I/O. Occurrence / syntactic identity is primary;
//! [`ResolvedCall::resolved_fqn`] is derived with explicit [`ResolveConfidence`].

mod call_resolve;

pub use call_resolve::{build_occurrence_fqn, resolve_file_calls, resolve_repo_calls, ResolveConfidence, ResolvedCall};
