# Go native adapter sidecar

Pinned `go/packages` + `go/types` worker. Speaks the shared Entmoot sidecar JSON
envelope on stdin/stdout (see `crates/entmoot-engine` `producers::sidecar`).

Multi-module trees (e.g. OpenTelemetry Demo with `src/*/go.mod`) are discovered
automatically: each `go.mod` under the request root is loaded with `./...`, and
call paths are relativized to that root.

Test variants are included by default (`ENTMOOT_GO_TESTS=include`); set `exclude`
for production-only native scope. The receipt records this choice, and both
Docker proxies forward it. Build tags apply to tests too. Repeated identical
production call facts coalesce; variant disagreements stay separate. Conversions
are intrinsic observations with no repository callable body. See the
[operator scope policy](../../../docs/runbooks/resolver-adapters.md#go).

**Cut (perf) (playbook §6):** default load today uses a heavy `Need*` set
(`LoadAllSyntax` …). When producing for a scoped package set, prefer smaller
`Need*` modes and mark coverage `partial` rather than always loading full syntax
for the whole module graph. See
[`docs/architecture/ARCHITECTURE.md`](../../../docs/architecture/ARCHITECTURE.md).

## Build (host Go)

```bash
cd tools/resolver-adapters/go
go build -o entmoot-go-resolver .
```

On Windows the artifact is `entmoot-go-resolver.exe`.

```bash
export ENTMOOT_GO_ADAPTER=/absolute/path/to/entmoot-go-resolver
```

If unset, the engine looks for this directory relative to `entmoot-engine`.

## Docker Compose toolchain (optional override)

Prefer the host Go build above. Use Docker only when the host has no suitable Go
(missing, too old, or wrong arch). Docker Desktop must be running.
`ENTMOOT_GO_VIA_DOCKER=1` is a non-default override — not the release path.

```bash
# once / when Dockerfile changes
docker compose -f tools/resolver-adapters/go/docker-compose.yml build

# point the producer at the Docker proxy binary (optional Windows escape hatch)
#   go build -C tools/resolver-adapters/go/docker_proxy -o ../entmoot-go-resolver-docker.exe .
export ENTMOOT_GO_ADAPTER="$PWD/tools/resolver-adapters/go/entmoot-go-resolver-docker"
# Windows:
#   set ENTMOOT_GO_ADAPTER=%CD%\tools\resolver-adapters\go\entmoot-go-resolver-docker.exe
#   set ENTMOOT_GO_VIA_DOCKER=1
# Optional Python proxy remains for debugging: docker_sidecar_proxy.py


entmoot-producer go produce --graph ... --root ... --out ...

# tear down image / module cache when finished
docker compose -f tools/resolver-adapters/go/docker-compose.yml down --rmi local
docker volume rm entmoot-go-modcache 2>/dev/null || true
```

PowerShell:

```powershell
docker compose -f tools/resolver-adapters/go/docker-compose.yml build
go build -C tools/resolver-adapters/go/docker_proxy -o ../entmoot-go-resolver-docker.exe .
$env:ENTMOOT_GO_ADAPTER = "$PWD\tools\resolver-adapters\go\entmoot-go-resolver-docker.exe"
# ... produce ...
docker compose -f tools/resolver-adapters/go/docker-compose.yml down --rmi local
docker volume rm entmoot-go-modcache
```

The proxy bind-mounts `request.root` at `/src` (stripping Windows `\\?\`
prefixes), prewarms `go mod download` inside the container (network allowed only
for prewarm; module cache persists in volume `entmoot-go-modcache`), then runs
the sidecar with `GOPROXY=off` for the typed load. Prefer
`entmoot-go-resolver-docker.exe` over the `.py` helper on Windows (stdin piping).

## Offline / proxy knobs

| Env | Default | Meaning |
|-----|---------|---------|
| `ENTMOOT_GO_GOPROXY` | `off` | Sidecar load proxy (Safe offline) |
| `ENTMOOT_GO_GOTOOLCHAIN` | `local` | Sidecar toolchain pin |
| `ENTMOOT_GO_PREWARM_GOPROXY` | `https://proxy.golang.org,direct` | Compose entrypoint download proxy |
| `ENTMOOT_GO_VIA_DOCKER` | unset | Dogfood sets solver to the compose proxy when `1` |
| `ENTMOOT_GO_TESTS` | `include` | `exclude` skips `*_test.go` (useful when test-only deps lack go.sum under Safe offline) |

Nested module roots named `_examples`, `examples`, or `example` are skipped so
demo apps under `GOPROXY=off` do not fail the primary inventory module.
