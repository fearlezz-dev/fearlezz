#!/bin/sh
# Installs the newest Fearlezz release into ~/.local/bin, or into $FEARLEZZ_INSTALL_DIR.
# Set FEARLEZZ_VERSION (for example v0.1.0) to install a specific release.
set -eu

repo="fearlezz-dev/fearlezz"
dir="${FEARLEZZ_INSTALL_DIR:-$HOME/.local/bin}"

case "$(uname -s)" in
  Linux) os="unknown-linux-musl" ;;
  Darwin) os="apple-darwin" ;;
  *) echo "fearlezz: no release for $(uname -s); build from source with cargo" >&2; exit 1 ;;
esac
case "$(uname -m)" in
  x86_64 | amd64) arch="x86_64" ;;
  arm64 | aarch64) arch="aarch64" ;;
  *) echo "fearlezz: no release for $(uname -m); build from source with cargo" >&2; exit 1 ;;
esac

name="fearlezz-$arch-$os"
if [ -n "${FEARLEZZ_VERSION:-}" ]; then
  base="https://github.com/$repo/releases/download/$FEARLEZZ_VERSION"
else
  base="https://github.com/$repo/releases/latest/download"
fi

tmp="$(mktemp -d)"
trap 'rm -r "$tmp"' EXIT
curl -fsSL "$base/$name.tar.gz" -o "$tmp/$name.tar.gz"
curl -fsSL "$base/$name.tar.gz.sha256" -o "$tmp/$name.tar.gz.sha256"
(cd "$tmp" && shasum -a 256 -c "$name.tar.gz.sha256" >/dev/null)
tar -xzf "$tmp/$name.tar.gz" -C "$tmp"
mkdir -p "$dir"
install -m 755 "$tmp/$name/fearlezz" "$dir/fearlezz"

echo "installed $("$dir/fearlezz" --version) to $dir/fearlezz"
case ":$PATH:" in
  *":$dir:"*) ;;
  *) echo "add $dir to your PATH" ;;
esac
