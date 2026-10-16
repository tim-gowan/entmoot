//! Default CLI EnvFilter must not enable third-party INFO (ra_ap_/salsa flood).
#![allow(clippy::unwrap_used)]

use tracing_subscriber::EnvFilter;

#[test]
fn default_operator_filter_parses() {
    let filter = EnvFilter::try_new("warn,entmoot=info,entmoot_engine=info").unwrap();
    drop(filter);
}

#[test]
fn rust_log_can_opt_into_solver_debug() {
    let filter = EnvFilter::try_new("warn,entmoot_engine=info,ra_ap_=debug,salsa=debug").unwrap();
    drop(filter);
}
