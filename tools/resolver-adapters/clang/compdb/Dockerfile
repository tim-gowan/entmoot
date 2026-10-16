# Curl CMake CompDB materialization (Linux). Host mounts source at /src.
# Operator-only — never invoked by entmoot init/bind.
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    ca-certificates \
    cmake \
    ninja-build \
    pkg-config \
    libssl-dev \
    zlib1g-dev \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY entrypoint.sh /usr/local/bin/entmoot-curl-compdb-entrypoint
RUN sed -i 's/\r$//' /usr/local/bin/entmoot-curl-compdb-entrypoint \
    && chmod +x /usr/local/bin/entmoot-curl-compdb-entrypoint
ENTRYPOINT ["/usr/local/bin/entmoot-curl-compdb-entrypoint"]
