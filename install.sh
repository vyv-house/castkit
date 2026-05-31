#!/bin/sh
set -e

fail() { printf '%s\n' "Error: $*" >&2; exit 1; }

os=$(uname -s 2>/dev/null || true)
[ "$os" = Darwin ] || fail "Unsupported OS: $os (macOS only)"

arch=$(uname -m 2>/dev/null || true)
case "$arch" in
  arm64) target=aarch64-apple-darwin ;;
  x86_64) target=x86_64-apple-darwin ;;
  *) fail "Unsupported architecture: $arch" ;;
esac

api='https://api.github.com/repos/vyvhouse/castkit/releases/latest'
tag=$(curl -fsSL "$api" | awk -F'"tag_name":"' 'NF>1{print $2}' | awk -F'"' 'NR==1{print $1}')
[ -n "$tag" ] || fail "Could not determine latest release tag"

url="https://github.com/vyvhouse/castkit/releases/download/$tag/castkit-$target.tar.gz"
tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/castkit.XXXXXX") || fail "Failed to create temp directory"
trap 'rm -rf "$tmpdir"' EXIT HUP INT TERM

archive="$tmpdir/castkit.tar.gz"
curl -fsSL "$url" -o "$archive" || fail "Download failed: $url"
tar -xzf "$archive" -C "$tmpdir" || fail "Failed to extract archive"

bindir=/usr/local/bin
if [ ! -w "$bindir" ]; then
  bindir="$HOME/.local/bin"
  mkdir -p "$bindir" || fail "Failed to create $bindir"
  case ":$PATH:" in *":$bindir:":*) ;; *) printf '%s\n' "Warning: $bindir is not on PATH" >&2 ;; esac
fi

mkdir -p "$bindir" || fail "Failed to create $bindir"
cp "$tmpdir/castkit" "$bindir/castkit" || fail "Failed to install castkit"
chmod 755 "$bindir/castkit"
PATH="$bindir:$PATH"; export PATH
castkit --version >/dev/null 2>&1 || fail "Installed binary did not run correctly"

printf '%s\n' "castkit installed to $bindir"
printf '%s\n' "Quick start: castkit share --room my-room --server wss://..."
