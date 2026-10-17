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

These settings require an existing crate. For the first publication, store a
short-lived `CARGO_REGISTRY_TOKEN` in the `crates-io` environment with
publish-new/publish-update access to the five names, and set
`CRATES_IO_TRUSTED_PUBLISHING=false`. Use the same tag-driven release workflow.
Once every crate exists, configure its Trusted Publisher, enable
`CRATES_IO_TRUSTED_PUBLISHING=true`, and revoke/remove the bootstrap token.

Docs: https://crates.io/docs/trusted-publishing

## How to cut a release

1. Bump `workspace.package.version` and exact internal dependency versions on the
   release branch. Let Cargo update the lockfile, then regenerate inputs with
   `python scripts/prepare_release_inputs.py` and follow the
   [validation gates](runbooks/local-alpha.md#validation-and-release-gates).
2. Tag and push `v<version>` (must match `Cargo.toml`, e.g. `v0.0.1-alpha`).
3. `release.yml` gates publication on ci / native / docs for that SHA, packages crates, creates the
   GitHub Release with `.crate` assets, publishes to crates.io, then runs the
   shared `docs.yml` workflow again for post-publish Pages when enabled. CI/PR docs
   stage under `.tmp/staging` via the same `docs.yml` defaults. `demo.yml` is
   opt-in corpus dispatch and is not part of Trusted Publishing or tag CD.
4. Approve the **`crates-io`** environment if reviewers are required.

Keep pushed release tags immutable. If a gate fails before publication, repair
the candidate and use a new prerelease version/tag. Reconcile any partial registry
publication before retrying; a version cannot be overwritten.

Pages requires an Actions-based Pages site and `PAGES_ENABLED=true`. Private
repositories need a supporting GitHub plan. Documentation qualification produces
a staging artifact even when Pages hosting is unavailable.

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
   ci.yml + native.yml + docs.yml staging (workflow_call)
   (demo.yml is dispatch-only; not on this path)
        │
        ▼
   release.yml publish job (environment crates-io):
        check_release --tag → refuse if crates.io has version →
        package_smoke → gh release create (.crate) →
        OIDC (or initial-publish token) → publish_release.py
        │
        ▼
   docs.yml (workflow_call, after publish):
        build → .tmp/site → Pages artifact → deploy if PAGES_ENABLED
```
