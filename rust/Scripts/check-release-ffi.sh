#!/usr/bin/env bash
# Host builds MUST use release-ffi (panic=unwind). This gate:
# 1) Builds ab-core with --profile release-ffi and checks the artifact exists.
# 2) Documents that `cargo build --release -p ab-core` is expected to fail
#    (compile_error on panic=abort) — optional verify via EXPECT_RELEASE_ABORT=1.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "==> cargo build --profile release-ffi -p ab-core"
cargo build --profile release-ffi -p ab-core

# Locate cdylib / staticlib produced for this host.
found=0
for candidate in \
  target/release-ffi/ab_core.dll \
  target/release-ffi/ab_core.lib \
  target/release-ffi/libab_core.a \
  target/release-ffi/libab_core.dylib \
  target/release-ffi/libab_core.so \
  target/release-ffi/deps/ab_core.dll \
  target/release-ffi/deps/libab_core.a
do
  if [[ -f "$candidate" ]]; then
    echo "OK artifact: $candidate"
    found=1
  fi
done
if [[ "$found" -eq 0 ]]; then
  echo "error: no ab_core library artifact under target/release-ffi" >&2
  ls -la target/release-ffi || true
  exit 1
fi

if [[ "${EXPECT_RELEASE_ABORT:-0}" == "1" ]]; then
  echo "==> expect cargo build --release -p ab-core to fail (panic=abort compile_error)"
  if cargo build --release -p ab-core 2>/dev/null; then
    echo "error: release build unexpectedly succeeded" >&2
    exit 1
  fi
  echo "OK release build failed as expected"
fi

echo "release-ffi gate passed"
