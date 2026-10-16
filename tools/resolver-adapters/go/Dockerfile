# Go toolchain + sidecar for Entmoot Class C produce (Linux amd64).
# Use when the host lacks a suitable Go (version/arch) or module cache.
FROM golang:1.26-bookworm AS build
WORKDIR /src
COPY go.mod go.sum ./
RUN go mod download
COPY *.go ./
RUN CGO_ENABLED=0 go build -o /out/entmoot-go-resolver .

FROM golang:1.26-bookworm
RUN apt-get update && apt-get install -y --no-install-recommends bash \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /out/entmoot-go-resolver /usr/local/bin/entmoot-go-resolver
COPY docker-entrypoint.sh /usr/local/bin/entmoot-go-entrypoint
RUN sed -i 's/\r$//' /usr/local/bin/entmoot-go-entrypoint \
    && chmod +x /usr/local/bin/entmoot-go-entrypoint /usr/local/bin/entmoot-go-resolver
WORKDIR /src
ENTRYPOINT ["/usr/local/bin/entmoot-go-entrypoint"]
