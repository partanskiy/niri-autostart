#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 <version> <repo-url>" >&2
  exit 1
fi

version=$1
repo_url=${2%/}
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

replace_line() {
  local pattern=$1
  local replacement=$2
  local file=$3

  sed -i -E "s|^${pattern}$|${replacement}|" "$file"
}

source_url="${repo_url}/archive/refs/tags/v${version}.tar.gz"
bin_x86_url="${repo_url}/releases/download/v${version}/niri-autostart-x86_64-linux.tar.gz"
bin_aarch64_url="${repo_url}/releases/download/v${version}/niri-autostart-aarch64-linux.tar.gz"

source_sha=$(download_and_sha256 "$source_url" "niri-autostart-source.tar.gz")
bin_x86_sha=$(download_and_sha256 "$bin_x86_url" "niri-autostart-x86_64-linux.tar.gz")
bin_aarch64_sha=$(download_and_sha256 "$bin_aarch64_url" "niri-autostart-aarch64-linux.tar.gz")

src_pkgbuild="$repo_root/aur/niri-autostart/PKGBUILD"
bin_pkgbuild="$repo_root/aur/niri-autostart-bin/PKGBUILD"

replace_line 'pkgver=.*' "pkgver=${version}" "$src_pkgbuild"
replace_line "sha256sums=\\('.*'\\)" "sha256sums=('${source_sha}')" "$src_pkgbuild"

replace_line 'pkgver=.*' "pkgver=${version}" "$bin_pkgbuild"
replace_line "sha256sums_x86_64=\\('.*'\\)" "sha256sums_x86_64=('${bin_x86_sha}')" "$bin_pkgbuild"
replace_line "sha256sums_aarch64=\\('.*'\\)" "sha256sums_aarch64=('${bin_aarch64_sha}')" "$bin_pkgbuild"

(
  cd "$repo_root/aur/niri-autostart"
  makepkg --printsrcinfo > .SRCINFO
)

(
  cd "$repo_root/aur/niri-autostart-bin"
  makepkg --printsrcinfo > .SRCINFO
)
