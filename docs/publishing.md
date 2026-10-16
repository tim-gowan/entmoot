# Publishing to crates.io

Secure release path for the five workspace crates via **Trusted Publishing
(OIDC)** (`rust-lang/crates-io-auth-action`). No long-lived
`CARGO_REGISTRY_TOKEN` is required when Trusted Publishing is configured.

## Threat model

| Risk | Mitigation |
|------|------------|
| Stolen long-lived crates.io token | OIDC short-lived token; revoked when the job ends |
| Publish from arbitrary workflows | Trusted Publisher binds owner + repo + **`release.yml`** + `crates-io` |
| Accidental publish | Tag-driven CD only; protected `crates-io` environment |
| Wrong version | `check_release.py --tag` asserts tag ↔ workspace version |
| Duplicate version | Release job refuses if any crate@version already exists on crates.io |

`repository` / `homepage` in `Cargo.toml` are display metadata only. Auth
coupling is the Trusted Publisher config—not the git remote URL.

## One-time setup

### 1. GitHub Environment

Environment name must be exactly `crates-io`. Recommended protections: required
reviewers; allow the tags used for release (`v*`).

### 2. crates.io Trusted Publisher (each crate)

For `entmoot-contracts`, `entmoot-core`, `entmoot-engine`, `entmoot`, and
`entmoot-mcp`:

- **Repository owner:** `tim-gowan`
- **Repository name:** `entmoot`
- **Workflow filename:** `release.yml` (not the full path)
- **Environment:** `crates-io`

Then set repository variable `CRATES_IO_TRUSTED_PUBLISHING=true`.

Docs: https://crates.io/docs/trusted-publishing

## How to cut a release

1. Bump `workspace.package.version` in `Cargo.toml` on `main` and land CI green.
2. Tag and push `v<version>` (must match `Cargo.toml`, e.g. `v0.0.1-alpha`).
3. `release.yml` runs ci / native / docs on that SHA, packages crates, creates the
   GitHub Release with `.crate` assets, then publishes to crates.io in dependency
   order and verifies a clean registry install.
4. Approve the **`crates-io`** environment if reviewers are required.

## Manual dry-run / emergency bootstrap

`publish.yml` is **not** the primary CD path. Use Actions → **Publish to crates.io
(dry-run / bootstrap)** with `publish: false` to validate packaging only.
`publish: true` is emergency crates.io bootstrap and does **not** create a GitHub
Release—prefer tagging for `release.yml`.

## Local emergency publish

Only if CI is unavailable. Prefer rotating any API token afterward.

```powershell
# load CARGO_REGISTRY_TOKEN from a local dotenv — never commit it
python scripts/publish_release.py
```

## Workflow map

```text
push tag v*
        │
        ▼
   ci.yml + native.yml + docs.yml (workflow_call)
        │
        ▼
   release.yml publish job (environment crates-io):
        check_release --tag → refuse if crates.io has version →
        package_smoke → gh release create (.crate) →
        crates-io-auth-action (OIDC) → publish_release.py
```
