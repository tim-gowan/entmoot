# Publishing to crates.io

Secure release path for `entmoot`: **Trusted Publishing (OIDC)** via the official
[`rust-lang/crates-io-auth-action`](https://github.com/rust-lang/crates-io-auth-action).
No long-lived `CARGO_REGISTRY_TOKEN` is stored in GitHub Actions secrets.

Toolchain for CI/publish jobs is the **Docker Official Image**
[`rust`](https://hub.docker.com/_/rust), pinned by digest in the workflows.

## Threat model (what this buys)

| Risk | Mitigation |
|------|------------|
| Stolen long-lived crates.io token in Actions secrets | OIDC short-lived token (~30 min), revoked when the job ends |
| Publish from arbitrary workflows | crates.io Trusted Publisher binds **owner + repo + workflow filename + environment** |
| Accidental publish | `workflow_dispatch` defaults `publish: false`; real upload needs the `crates-io` environment |
| Dirty / wrong version | Gate job asserts `Cargo.toml` version, runs fmt/clippy/test/`cargo publish --dry-run` |
| Duplicate version | Gate refuses if `entmoot@VERSION` already exists on crates.io |

`repository` / `homepage` in `Cargo.toml` are **display metadata only**. Auth coupling is the Trusted Publisher config below—not the git remote URL.

## One-time setup

### 1. GitHub Environment

Environment name must be exactly `crates-io` (matches the workflow).

Recommended protections (Settings → Environments → `crates-io`):

- Required reviewers (yourself)
- Deployment branches: protected branches only (`main`), **or** selected tags if you publish from `v*` tag pushes

### 2. crates.io Trusted Publisher

1. Open https://crates.io/crates/entmoot/settings
2. **Trusted Publishing** → add GitHub Actions publisher:
   - **Repository owner:** `tim-gowan`
   - **Repository name:** `entmoot`
   - **Workflow filename:** `publish.yml` (not the full path)
   - **Environment:** `crates-io`
3. After the first successful OIDC publish, optionally enable **Trusted Publishing only** so API tokens cannot publish new versions.

Docs: https://crates.io/docs/trusted-publishing

## How to cut a release

1. Bump `version` in `Cargo.toml` on `main` (and commit).
2. Actions → **Publish to crates.io** → Run workflow:
   - `version`: must match `Cargo.toml` (e.g. `0.0.2`)
   - `publish`: `false` for a dry-run gate only; `true` to upload
3. Approve the **`crates-io`** environment deployment if reviewers are required.
4. Confirm https://crates.io/crates/entmoot

Tag alternative: push `vX.Y.Z` (must match `Cargo.toml`); that path sets `publish=true` automatically.

## Local emergency publish

Only if CI is unavailable. Prefer rotating the token afterward.

```powershell
cd G:\GitHub\entmoot
# load CARGO_REGISTRY_TOKEN from a local dotenv — never commit it
cargo publish
```

## Workflow map

```text
workflow_dispatch / tag v*
        │
        ▼
   classify (version + publish flag)
        │
        ▼
   gate (rust container): fmt → clippy → test → dry-run → exists-check
        │
        │  publish=true
        ▼
   publish (environment crates-io):
        checkout → crates-io-auth-action (OIDC) → cargo publish → API verify
```
