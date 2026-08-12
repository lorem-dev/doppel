# Troubleshooting

Fixes for problems that are not configuration mistakes. A configuration Doppel
refuses reports why and where; this page is for the cases where nothing is
reported because nothing got far enough to report it.

## Installing

### macOS refuses to run the downloaded binary

```
"doppel" cannot be opened because the developer cannot be verified.
```

or, from a shell:

```
zsh: killed     doppel
```

macOS attaches `com.apple.quarantine` to anything a browser downloads, and
Gatekeeper refuses a quarantined binary that is not signed with an Apple
Developer ID. The releases are not signed or notarized, so a copy downloaded
through a browser is rejected. The file is not damaged.

Clear the attribute:

```bash
xattr -d com.apple.quarantine ~/Downloads/doppel
```

Confirm that was the cause:

```bash
xattr -l ~/Downloads/doppel | grep quarantine
```

On Apple Silicon, if it still will not run after that -- an arm64 binary needs
at least an ad-hoc signature -- re-sign it locally and clear the attribute
again:

```bash
codesign --force --sign - ~/Downloads/doppel
xattr -d com.apple.quarantine ~/Downloads/doppel
```

**Avoiding it entirely:** the [one-line installer](installation.md#one-line)
downloads with `curl`, which does not set the attribute. Nothing needs
clearing after it.

### `doppel: command not found` after installing

The install directory is not on `PATH`. The installer prints which shell
profile it appended to; either open a new shell or source that file:

```bash
source ~/.zshrc
```

If it reported that it could not find a profile, add the directory yourself:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

### The installer says there is no prebuilt binary

Three platforms are built: macOS on Apple Silicon, and Linux on x86-64 and
arm64. Anywhere else, build from source -- the workspace is pure Rust and
needs no system libraries:

```bash
cargo install --path crates/doppel-cli
```

## Running

### `cannot bind 127.0.0.1:8080: Address already in use`

Something already holds the port. Find it:

```bash
lsof -nP -iTCP:8080 -sTCP:LISTEN
```

A previous `doppel` that did not shut down cleanly is the usual answer.

### `Permission denied` binding a port below 1024

Ports 1 to 1023 need elevated privilege. Doppel warns about one at startup
rather than refusing it, because running behind a capability or a redirect is
a legitimate deployment -- but if it was a typo, this is the error it produces.

On Linux, grant the capability instead of running as root:

```bash
sudo setcap 'cap_net_bind_service=+ep' "$(command -v doppel)"
```

### `the database schema is not ready`

The PostgreSQL store refuses to serve against an unmigrated schema rather than
altering it at startup. Apply the migrations, which is a separate, deliberate
step:

```bash
doppel config migrate --database-url "$DOPPEL_DATABASE_URL"
```

`doppel config migrate --status` reports what is applied without changing
anything. See [Storing configuration in PostgreSQL](storage.md#migrations).

### `control socket directory ... does not exist`

`control.socket` names a path whose parent directory is not there. Doppel
creates the socket, not the directory above it -- creating directories a
configuration names would let a typo silently produce a socket nobody is
looking for.

### A reload says sections need a restart

```
note: these sections changed but only take effect after a restart: server, admin
```

The listeners those sections describe are already bound. `admin.tokens` and
`admin.access` do take effect on reload; the host, the port and
`admin.enable` do not. See
[Changing configuration while it runs](runtime-changes.md).

## The admin API

### Every request answers 401

The token is not one the *running* configuration knows. Two common causes:

- It was written into the configuration file but not reloaded. Authorization
  is judged against what the last reload put into force, deliberately -- see
  [the admin API](admin-api.md#authentication).
- It is shadowed by `DOPPEL_ADMIN_TOKENS`. A token name claimed by the
  environment replaces the configured one entirely; startup logs a warning
  naming each one shadowed. See
  [Tokens from the environment](configuration.md#tokens-from-the-environment).

### A write answers 409

Someone changed that proxy between your read and your write. Read it again,
reapply the change, and retry with the new revision -- do not drop the
`If-Match` header, which is what makes the collision visible at all.

### `token add` says the name is supplied by the environment

A token issued under a name the environment already claims would never
authenticate, because the environment is searched first. Pick another name, or
remove it from `DOPPEL_ADMIN_TOKENS`.

---

Anything not covered here is worth an issue on the
[tracker](https://github.com/lorem-dev/doppel/issues).

## The dashboard

### The admin root answers 503

`DASHBOARD_NOT_BUILT`. This binary was compiled without the dashboard's static
assets -- a source build that skipped the frontend. Released binaries and the
published image always carry it.

```bash
npm --prefix frontend ci && npm --prefix frontend run build
cargo build --release -p doppel-cli
```

The order is the whole answer: the assets are embedded at compile time, so
building them without rebuilding the binary changes nothing about what it serves.
If Node is not available and the JSON API is enough, `admin.dashboard: false`
stops the route being served at all, which is a clearer answer than a 503.

### The page loads but every button is disabled

The caller has no rights to spend. The dashboard asks `GET /api/v1/access` what it
may do and disables what the server would refuse; the tooltip on each control says
so. Enter a token, or check `admin.access` -- the same two causes as a 401 above,
since it is the same decision.

### A change made elsewhere is not on screen

It will be within a minute: the list refetches on a timer, and the timer is paused
while the tab is hidden. Switching back to the tab refetches immediately.
