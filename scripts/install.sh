#!/bin/sh
# Doppel CLI installer for macOS and Linux.
#
# One line, nothing to set up first -- uses the curl/wget and tar already on
# the system:
#
#   curl -fsSL https://raw.githubusercontent.com/lorem-dev/doppel/main/scripts/install.sh | sh
#
# Environment overrides:
#   DOPPEL_VERSION      release tag to install (default: latest)
#   DOPPEL_INSTALL_DIR  where to put the binary (default: $HOME/.local/bin)
#
# Downloading with curl or wget rather than a browser matters on macOS: a
# browser attaches `com.apple.quarantine`, and an unsigned binary carrying it
# is refused by Gatekeeper. Nothing here sets that attribute, so a binary
# installed this way runs. See docs/usage/troubleshooting.md for a download
# that already went through a browser.
set -eu

REPO="lorem-dev/doppel"
BIN="doppel"
INSTALL_DIR="${DOPPEL_INSTALL_DIR:-$HOME/.local/bin}"
VERSION="${DOPPEL_VERSION:-latest}"

err() {
  printf 'error: %s\n' "$1" >&2
  exit 1
}

# Map the platform onto the Rust target triple the release assets are named
# with (doppel-<target>.tar.gz). Only the three that are built are accepted;
# anything else says so and names the way out rather than downloading a
# 404 page and failing to untar it.
os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
  Darwin)
    case "$arch" in
      arm64 | aarch64) target="aarch64-apple-darwin" ;;
      x86_64)
        err "no prebuilt binary for Intel macOS; build from source with: cargo install --path crates/doppel-cli"
        ;;
      *) err "unsupported macOS architecture: $arch" ;;
    esac
    ;;
  Linux)
    case "$arch" in
      x86_64 | amd64) target="x86_64-unknown-linux-gnu" ;;
      aarch64 | arm64) target="aarch64-unknown-linux-gnu" ;;
      *) err "no prebuilt binary for Linux $arch; build from source with: cargo install --path crates/doppel-cli" ;;
    esac
    ;;
  *)
    err "unsupported operating system: $os"
    ;;
esac

asset="${BIN}-${target}.tar.gz"

# `releases/latest/download/<asset>` always redirects to the newest release, so
# the common case needs no API call, no token and no jq.
if [ "$VERSION" = "latest" ]; then
  url="https://github.com/${REPO}/releases/latest/download/${asset}"
else
  url="https://github.com/${REPO}/releases/download/${VERSION}/${asset}"
fi

if command -v curl >/dev/null 2>&1; then
  download() { curl -fsSL "$1" -o "$2"; }
elif command -v wget >/dev/null 2>&1; then
  download() { wget -qO "$2" "$1"; }
else
  err "need curl or wget on PATH to download the release"
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

printf 'Downloading %s ...\n' "$asset"
download "$url" "$tmp/$asset" || err "download failed: $url"

printf 'Extracting ...\n'
tar -xzf "$tmp/$asset" -C "$tmp" || err "could not extract $asset"
[ -f "$tmp/$BIN" ] || err "the archive did not contain a '$BIN' binary"

# Verify against checksums.txt when it is published alongside, and say so
# either way -- a silent skip would read as a check that passed.
sums_url="${url%/*}/checksums.txt"
if download "$sums_url" "$tmp/checksums.txt" 2>/dev/null; then
  # `--ignore-missing` because checksums.txt covers every asset in the release
  # and only one of them was downloaded. Both implementations support it:
  # coreutils since 8.25, Perl shasum since 6.02.
  if command -v shasum >/dev/null 2>&1; then
    sha="shasum -a 256"
  elif command -v sha256sum >/dev/null 2>&1; then
    sha="sha256sum"
  else
    sha=""
  fi

  if [ -z "$sha" ]; then
    printf 'No shasum or sha256sum on PATH; skipping checksum verification.\n'
  elif (cd "$tmp" && $sha -c checksums.txt --ignore-missing >/dev/null 2>&1); then
    printf 'Checksum verified.\n'
  else
    err "checksum mismatch for $asset -- refusing to install"
  fi
else
  printf 'No checksums.txt published for this release; skipping verification.\n'
fi

mkdir -p "$INSTALL_DIR"
if command -v install >/dev/null 2>&1; then
  install -m 0755 "$tmp/$BIN" "$INSTALL_DIR/$BIN"
else
  cp "$tmp/$BIN" "$INSTALL_DIR/$BIN"
  chmod 0755 "$INSTALL_DIR/$BIN"
fi
printf 'Installed %s to %s\n' "$BIN" "$INSTALL_DIR/$BIN"

# Put INSTALL_DIR on PATH by appending to the first shell profile that exists,
# and only when it is not already there.
case ":${PATH}:" in
  *":${INSTALL_DIR}:"*)
    : # already on PATH
    ;;
  *)
    added=""
    for rc in "$HOME/.zshrc" "$HOME/.bashrc" "$HOME/.profile"; do
      if [ -f "$rc" ]; then
        if ! grep -qsF "$INSTALL_DIR" "$rc"; then
          # shellcheck disable=SC2016  # `$PATH` must stay literal here
          #
          # The single quotes are the point: `%s` takes the install directory
          # now, and `$PATH` is written into the profile unexpanded so it
          # resolves when that profile is sourced. Expanding it here would
          # freeze today's PATH into the file.
          printf '\n# Added by the Doppel CLI installer\nexport PATH="%s:$PATH"\n' "$INSTALL_DIR" >> "$rc"
          printf 'Added %s to PATH in %s -- restart your shell to pick it up.\n' "$INSTALL_DIR" "$rc"
        fi
        added="yes"
        break
      fi
    done
    if [ -z "$added" ]; then
      printf 'Add %s to your PATH to run "%s" from anywhere.\n' "$INSTALL_DIR" "$BIN"
    fi
    ;;
esac

printf 'Done. '
"$INSTALL_DIR/$BIN" version || true
