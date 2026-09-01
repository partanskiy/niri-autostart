#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <binary>" >&2
  exit 2
fi

binary=$1

if [[ ! -f $binary ]]; then
  echo "binary not found: $binary" >&2
  exit 1
fi

if ! readelf -hW "$binary" >/dev/null; then
  echo "not a readable ELF binary: $binary" >&2
  exit 1
fi

if readelf -lW "$binary" | grep -q '[[:space:]]INTERP[[:space:]]'; then
  echo "binary has a dynamic program interpreter: $binary" >&2
  exit 1
fi

if readelf -dW "$binary" 2>/dev/null | grep -q '(NEEDED)'; then
  echo "binary has dynamic library dependencies: $binary" >&2
  exit 1
fi

echo "verified static ELF binary: $binary"
