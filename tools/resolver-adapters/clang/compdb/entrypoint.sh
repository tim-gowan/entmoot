#!/usr/bin/env bash
# Configure curl with CMake+Ninja and emit compile_commands.json under /src/build.
set -euo pipefail
ROOT="${ENTMOOT_COMPDB_SRC:-/src}"
BUILD="${ROOT}/build"
if [[ ! -f "${ROOT}/CMakeLists.txt" ]]; then
  echo "error: CMakeLists.txt missing at ${ROOT}" >&2
  exit 1
fi
mkdir -p "${BUILD}"
cmake -S "${ROOT}" -B "${BUILD}" \
  -G Ninja \
  -DCMAKE_EXPORT_COMPILE_COMMANDS=ON \
  -DCMAKE_BUILD_TYPE=Debug \
  -DBUILD_TESTING=OFF \
  -DCURL_USE_LIBPSL=OFF \
  -DCURL_DISABLE_TESTS=ON
# Do not require a full build for CompDB emission; configure is enough.
if [[ ! -f "${BUILD}/compile_commands.json" ]]; then
  echo "error: CMake did not emit ${BUILD}/compile_commands.json" >&2
  exit 1
fi
cp -f "${BUILD}/compile_commands.json" "${ROOT}/compile_commands.json"
echo "CompDB ready: ${BUILD}/compile_commands.json ($(wc -c < "${BUILD}/compile_commands.json") bytes)"
