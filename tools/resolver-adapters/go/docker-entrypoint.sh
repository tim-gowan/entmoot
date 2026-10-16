#!/bin/bash
# Prewarm module caches under /src (GOPROXY on), then run the offline sidecar.
set -eu
if [[ -d /src ]]; then
  while IFS= read -r -d '' mod; do
    dir=$(dirname "$mod")
    (
      cd "$dir"
      GOPROXY="${ENTMOOT_GO_PREWARM_GOPROXY:-https://proxy.golang.org,direct}" \
        GOTOOLCHAIN=local \
        GOFLAGS=-buildvcs=false \
        go mod download
    ) || true
  done < <(find /src -name go.mod -print0 2>/dev/null || true)
fi
exec /usr/local/bin/entmoot-go-resolver
