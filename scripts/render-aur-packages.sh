#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "usage: $0 <version> <repo-url> <output-dir>" >&2
  exit 1
fi

version=$1
repo_url=${2%/}
output_root=$3
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
tmpdir=$(mktemp -d)

cleanup() {
  rm -rf "$tmpdir"
}

trap cleanup EXIT

download_and_sha256() {
  local url=$1
  local output=$2

  curl -LfsS "$url" -o "$tmpdir/$output"
  sha256sum "$tmpdir/$output" | awk '{ print $1 }'
}

generate_srcinfo() {
  local pkgdir=$1

  if [[ $EUID -eq 0 ]]; then
    chown -R nobody:nobody "$pkgdir"
    runuser -u nobody -- sh -lc "cd '$pkgdir' && makepkg --printsrcinfo > .SRCINFO"
  else
    (
      cd "$pkgdir"
      makepkg --printsrcinfo > .SRCINFO
    )
  fi
}

render_template() {
  local template=$1
  local output=$2

  sed \
    -e "s|@PKGVER@|$version|g" \
    -e "s|@REPO_URL@|$repo_url|g" \
    -e "s|@SOURCE_URL@|$source_url|g" \
    -e "s|@SOURCE_SHA256@|$source_sha|g" \
    -e "s|@BIN_X86_URL@|$bin_x86_url|g" \
    -e "s|@BIN_AARCH64_URL@|$bin_aarch64_url|g" \
    -e "s|@BIN_X86_SHA256@|$bin_x86_sha|g" \
    -e "s|@BIN_AARCH64_SHA256@|$bin_aarch64_sha|g" \
    "$template" > "$output"
}

source_url="${repo_url}/archive/refs/tags/v${version}.tar.gz"
bin_x86_url="${repo_url}/releases/download/v${version}/niri-autostart-x86_64-linux.tar.gz"
bin_aarch64_url="${repo_url}/releases/download/v${version}/niri-autostart-aarch64-linux.tar.gz"

source_sha=$(download_and_sha256 "$source_url" "niri-autostart-source.tar.gz")
bin_x86_sha=$(download_and_sha256 "$bin_x86_url" "niri-autostart-x86_64-linux.tar.gz")
bin_aarch64_sha=$(download_and_sha256 "$bin_aarch64_url" "niri-autostart-aarch64-linux.tar.gz")

mkdir -p "$output_root/niri-autostart" "$output_root/niri-autostart-bin"

render_template \
  "$repo_root/aur/niri-autostart/PKGBUILD.in" \
  "$output_root/niri-autostart/PKGBUILD"

render_template \
  "$repo_root/aur/niri-autostart-bin/PKGBUILD.in" \
  "$output_root/niri-autostart-bin/PKGBUILD"

generate_srcinfo "$output_root/niri-autostart"
generate_srcinfo "$output_root/niri-autostart-bin"
