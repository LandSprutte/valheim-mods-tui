#!/usr/bin/env bash
#
# Build shippable release binaries for every platform valheim-mods-tui targets.
#
# Usage:
#   ./scripts/build-release.sh                    # build every target for this host
#   ./scripts/build-release.sh x86_64-pc-windows-gnu
#   ./scripts/build-release.sh --tag v0.2.0       # build, then publish a GitHub release
#
# Output lands in dist/ as valheim-mods-tui-<target-triple> (plus .exe on
# Windows), the exact names `valheim-mods-tui --update` looks for, alongside a
# SHA256SUMS file.
#
# Linux and Windows targets are cross-built with cargo-zigbuild so they are
# self-contained static binaries. Apple targets use the host toolchain and its
# SDK and can only be produced on macOS.
set -euo pipefail

cd "$(dirname "$0")/.."

# The full matrix we publish. aarch64/x86_64 covers Apple Silicon vs Intel Macs
# and arm vs x86 Linux; Windows is cross-built as a self-contained .exe.
ALL_TARGETS=(
  x86_64-unknown-linux-musl
  aarch64-unknown-linux-musl
  aarch64-apple-darwin
  x86_64-apple-darwin
  x86_64-pc-windows-gnu
)

TAG=""
TARGETS=()
while [ $# -gt 0 ]; do
  case "$1" in
    --tag)
      [ $# -ge 2 ] || { echo "error: --tag needs a value" >&2; exit 2; }
      TAG="$2"
      shift 2
      ;;
    -h|--help)
      sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    -*)
      echo "error: unknown flag $1" >&2
      exit 2
      ;;
    *)
      TARGETS+=("$1")
      shift
      ;;
  esac
done

[ ${#TARGETS[@]} -gt 0 ] || TARGETS=("${ALL_TARGETS[@]}")

host_os="$(uname -s)"
version="$(cargo metadata --no-deps --format-version 1 \
  | sed -n 's/.*"name":"valheim-mods-tui","version":"\([^"]*\)".*/\1/p')"
version="${version:-unknown}"

need_zig() {
  if ! command -v cargo-zigbuild >/dev/null 2>&1 || ! command -v zig >/dev/null 2>&1; then
    cat >&2 <<'EOF'
error: cross-built targets need zig and cargo-zigbuild.
  brew install zig            # or: pip install ziglang
  cargo install cargo-zigbuild
EOF
    exit 1
  fi
}

ensure_target() {
  rustup target list --installed | grep -qx "$1" || {
    echo "adding Rust target $1"
    rustup target add "$1"
  }
}

echo "valheim-mods-tui $version — building ${#TARGETS[@]} target(s) on $host_os"

rm -rf dist
mkdir -p dist

for target in "${TARGETS[@]}"; do
  bin="valheim-mods-tui"
  asset="valheim-mods-tui-$target"
  case "$target" in
    *-windows-*)
      need_zig
      ensure_target "$target"
      echo "==> $target (cargo zigbuild)"
      cargo zigbuild --release --target "$target"
      bin="valheim-mods-tui.exe"
      asset="$asset.exe"
      ;;
    *-linux-musl)
      need_zig
      ensure_target "$target"
      echo "==> $target (cargo zigbuild)"
      cargo zigbuild --release --target "$target"
      ;;
    *-apple-darwin)
      if [ "$host_os" != "Darwin" ]; then
        echo "skip $target: Apple targets require a macOS host" >&2
        continue
      fi
      ensure_target "$target"
      echo "==> $target (cargo build)"
      cargo build --release --target "$target"
      ;;
    *)
      ensure_target "$target"
      echo "==> $target (cargo build)"
      cargo build --release --target "$target"
      ;;
  esac

  cp "target/$target/release/$bin" "dist/$asset"
  chmod +x "dist/$asset"
  echo "    -> dist/$asset"
done

# Checksums so downloaders can verify what they fetched.
if [ "$host_os" = "Darwin" ]; then
  (cd dist && shasum -a 256 valheim-mods-tui-* > SHA256SUMS)
else
  (cd dist && sha256sum valheim-mods-tui-* > SHA256SUMS)
fi

echo
echo "Built $version into dist/:"
ls -lh dist

if [ -n "$TAG" ]; then
  command -v gh >/dev/null 2>&1 || { echo "error: --tag needs the gh CLI" >&2; exit 1; }
  [ -n "${GH_TOKEN:-}${GITHUB_TOKEN:-}" ] || gh auth status >/dev/null 2>&1 \
    || { echo "error: gh is not authenticated (run: gh auth login)" >&2; exit 1; }
  echo
  echo "Publishing release $TAG …"
  gh release create "$TAG" --title "$TAG" --generate-notes \
    dist/valheim-mods-tui-* dist/SHA256SUMS || \
  gh release upload "$TAG" --clobber dist/valheim-mods-tui-* dist/SHA256SUMS
  echo "Published $TAG."
else
  echo
  echo "Tag a commit to publish automatically, or run:"
  echo "  ./scripts/build-release.sh --tag v$version"
fi
