# Third-party notices for optional Resolver adapter dependencies

The `resolved-rust-analyzer` feature of `entmoot-engine` (consumed only by
`entmoot-producer`) embeds aligned `ra_ap_*` crates published from the
rust-analyzer project.

- Project: https://github.com/rust-lang/rust-analyzer
- License: MIT OR Apache-2.0
- Pin surface: `crates/entmoot-engine/Cargo.toml` + `RA_AP_PIN`
- Update helper: `scripts/update_ra_ap_pins.py`

CodeQL’s Rust extractor is a comparative reference for orchestration strategy
(build-mode-none workspace loading). Entmoot does not copy CodeQL source.
