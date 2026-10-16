# entmoot_lints

Project-policy [Dylint](https://github.com/trailofbits/dylint) library for Entmoot.

| Lint | Policy |
| --- | --- |
| `banned_identity_digest` | No `sha2` / `sha256` paths; identity digests use BLAKE3 |
| `banned_sync_byte_mutex` | No `Mutex<Arc<[u8]>>` / `Mutex<Arc<Vec<u8>>>` |
| `anyhow_surface_only` | `anyhow` only in `entmoot-cli` / `entmoot-mcp` |
| `contracts_no_vendor_syntax` | No `tree_sitter*` / `rusqlite` imports in `entmoot-contracts` |

Cargo.toml / Cargo.lock `sha2` dependency bans remain in `scripts/pre_commit_check.py` (Dylint does not read manifests).

```bash
cargo dylint --all --workspace --fail-on-no-libraries
```

On Windows, pre-commit maps the repo to a short drive letter before running
Dylint so grammar `OUT_DIR` paths stay under `MAX_PATH`.

This package is not a workspace member (`workspace.exclude`); it uses the
toolchain in `rust-toolchain.toml`.
