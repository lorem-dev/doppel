# Installing

## One line

```bash
curl -fsSL https://raw.githubusercontent.com/lorem-dev/doppel/main/scripts/install.sh | sh
```

Detects the platform, downloads the matching archive from the latest release,
verifies it against the release's `checksums.txt`, installs `doppel` into
`~/.local/bin`, and adds that directory to your shell profile if it is not
already on `PATH`.

Two environment variables change what it does:

| Variable | Effect |
|---|---|
| `DOPPEL_VERSION` | Install a specific tag instead of the latest, e.g. `v0.2.0` |
| `DOPPEL_INSTALL_DIR` | Install somewhere other than `~/.local/bin` |

```bash
DOPPEL_VERSION=v0.2.0 DOPPEL_INSTALL_DIR=/usr/local/bin \
  curl -fsSL https://raw.githubusercontent.com/lorem-dev/doppel/main/scripts/install.sh | sh
```

A checksum mismatch stops the install and leaves nothing behind, rather than
warning and continuing.

## Prebuilt binaries

Every release publishes these. Any other platform has to be built from source.

| Platform | Archive |
|---|---|
| macOS, Apple Silicon | `doppel-aarch64-apple-darwin.tar.gz` |
| Linux, x86-64 | `doppel-x86_64-unknown-linux-gnu.tar.gz` |
| Linux, arm64 | `doppel-aarch64-unknown-linux-gnu.tar.gz` |

Each archive holds one file: the `doppel` binary.

!!! warning "Downloading through a browser on macOS"
    A browser attaches `com.apple.quarantine` to whatever it downloads, and
    macOS refuses to run an unsigned binary carrying it. The one-line
    installer uses `curl`, which does not set that attribute, so it is not
    affected -- but a manual download from the releases page is. See
    [Troubleshooting](troubleshooting.md#macos-refuses-to-run-the-downloaded-binary).

Verify a manual download against the release's `checksums.txt`:

```bash
shasum -a 256 -c checksums.txt --ignore-missing
```

## From source

The toolchain is pinned in `rust-toolchain.toml`, so rustup fetches the right
one:

```bash
cargo install --path crates/doppel-cli     # into ~/.cargo/bin
```

or build without installing:

```bash
cargo build --release -p doppel-cli        # target/release/doppel
```

## Confirming it worked

```bash
doppel version
```

If the command is not found, the install directory is not on your `PATH`. The
installer says which profile it wrote to; open a new shell, or source that
file.

## Next

[Getting started](getting-started.md) runs it against a real backend.
