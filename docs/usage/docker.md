# Running in Docker

Images are published to Docker Hub as
[`loremdev/doppel`](https://hub.docker.com/r/loremdev/doppel), for
`linux/amd64` and `linux/arm64`.

## Tags

| Tag | Moves? | Points at |
|---|---|---|
| `1.2.3-alpine` | no | exactly that release |
| `1.2-alpine` | yes | the newest patch of 1.2 |
| `1-alpine` | yes | the newest minor of 1 |

Every tag names its base. That is the convention the images this one sits
beside use -- `postgres:17.6-alpine`, `node:22-alpine` -- and it leaves the
unsuffixed names free, so a second base could be published later without
renaming anything anyone has already pinned.

There is deliberately **no `latest`**. A tag that moves is one an unpinned
deployment follows into a release nobody reviewed. Pin the full version in
anything that matters; `1.2-alpine` is a reasonable compromise for a
development environment.

A pre-release (`1.2.3-rc.1`) is published as `1.2.3-rc.1-alpine` only. It never
takes `1.2-alpine` or `1-alpine` with it.

## Running it

The image ships no configuration. That is deliberate: a default that quietly
proxied somewhere unintended would be worse than a container that refuses to
start.

```bash
docker run --rm \
  -p 8080:8080 -p 8081:8081 \
  -v "$PWD/config:/etc/doppel" \
  -v doppel-templates:/var/lib/doppel/templates \
  loremdev/doppel:1.2.3-alpine
```

Two mounts, and the second is not optional if you use templates at all --
see [Templates](#templates) below.

**Mount the directory, not the file.** `-v "$PWD/main.yaml:/etc/doppel/main.yaml"`
looks tidier and breaks every write: a save puts the new configuration in a
temporary file and renames it over the old one, and nothing can rename over a mount
point. Reads work, so the dashboard lists proxies happily and then refuses to save
one with `Resource busy (os error 16)`. With the directory mounted, the
configuration inside it is an ordinary file and the rename stays within the mount.

Add `:ro` to the mount if nothing should write the configuration -- then the admin
API's writes are refused for a reason it can state.

The configuration must bind `0.0.0.0` rather than `127.0.0.1`, or nothing
outside the container can reach it:

```yaml
server:
  host: "0.0.0.0"
  port: 8080
admin:
  host: "0.0.0.0"
  port: 8081
```

## Templates

**A volume is required.** Templates uploaded through the admin API are written
to disk at the moment of upload, and the render path reads them from disk per
request. Without a volume they live in the container's writable layer and go
with it -- `docker stop` and `docker run` again, and every uploaded template is
gone. Nothing reports this: the mock that named the file answers with a render
error on the next request, which reads like a broken template rather than a
missing volume.

```bash
docker volume create doppel-templates
docker run --rm \
  -p 8080:8080 -p 8081:8081 \
  -v "$PWD/config:/etc/doppel" \
  -v doppel-templates:/var/lib/doppel/templates \
  loremdev/doppel:1.2.3-alpine
```

`docker volume create` is optional -- `docker run` creates a named volume that
does not exist -- but naming it deliberately is how you find it again.

`templates.dir` has to point at the mount:

```yaml
templates:
  dir: /var/lib/doppel/templates
```

A relative `./templates`, as in `main.example.yaml`, also works: the image's
working directory is `/var/lib/doppel`, so it resolves to the same place.

## Reloading

`doppel config reload` talks to the control socket, which lives inside the
container. Two ways in:

```bash
# Through the admin API, if it is enabled and you hold a token.
curl -X POST -H "X-Proxy-Authorization: Bearer $TOKEN" \
     http://localhost:8081/api/v1/config/reload

# Or the same binary, inside the container.
docker exec <container> doppel config reload --socket /tmp/doppel.sock
```

Editing `config/main.yaml` on the host changes the file the container reads, so a
reload picks it up without restarting anything. It works the other way too: a write
through the admin API rewrites that file, in canonical form -- so comments in it do
not survive an edit made from the dashboard.

## Tokens

`DOPPEL_ADMIN_TOKENS` is the natural fit here -- it keeps secrets out of the
configuration file that gets mounted in:

```bash
docker run --rm \
  -e DOPPEL_ADMIN_TOKENS='{"ci":{"token":"...","group":"admin"}}' \
  ...
```

See [Tokens from the environment](configuration.md#tokens-from-the-environment).

## Signals

The image runs Doppel under `tini`, so `docker stop` delivers `SIGTERM` and the
graceful drain runs: in-flight requests finish, then the process exits. Without
an init, a PID 1 with no default handler for the signal would be killed after
the timeout instead.

## Compose

```yaml
services:
  doppel:
    image: loremdev/doppel:1.2.3-alpine
    ports:
      - "8080:8080"
      - "8081:8081"
    volumes:
      - ./config:/etc/doppel
      - doppel-templates:/var/lib/doppel/templates
    environment:
      DOPPEL_ADMIN_TOKENS: '{"ci":{"token":"...","group":"admin"}}'
    healthcheck:
      # `/api/v1/status` needs no token by default and answers only once the runtime
      # is compiled and both listeners are bound. `-f` matters: without it
      # curl exits 0 on a 500 and the check passes for answering at all rather
      # than for answering correctly. The port is the one inside the
      # container.
      test: ["CMD", "curl", "-fsS", "-o", "/dev/null", "http://127.0.0.1:8081/api/v1/status"]
      interval: 5s
      timeout: 3s
      start_period: 5s
      retries: 5

volumes:
  doppel-templates:
```

A configuration with `admin.enable: false` has nothing to probe. Drop the
healthcheck there rather than pointing it at the proxy port, where every path
either reaches an upstream or is answered by a mock -- neither of which says
anything about whether Doppel itself is well.

## What the image contains

Alpine, `ca-certificates`, `tini`, and one statically linked binary. It runs as
a non-root user, so a configuration asking for a port below 1024 fails at
bind -- which is the right answer in a container; publish a low port on the
host instead.

`ca-certificates` is not optional: without a trust store a proxy cannot verify
an `https` upstream, and every request fails with `UPSTREAM_ERROR`.

## Building it yourself

```bash
make image
```

That builds the dashboard, then the image. Nothing else is needed and no Rust
toolchain has to be able to target Linux: when `dist/` holds no binary the
Dockerfile compiles one in its own builder stage, for the platform being built,
and the builder stage is discarded -- the published image is Alpine plus the
binary, around 65 MB.

`make image-rebuild` does the same with no npm or docker cache, and with `dist/`
emptied first so the compile definitely happens.

### Why it prefers a staged binary

`docker build .` uses `dist/<platform>/doppel`, or `dist/doppel`, if either is
there, and compiles only when neither is. That preference is what keeps releases
fast: building Rust inside a multi-architecture `buildx` means QEMU for the
non-native architecture, which turns a two-minute compile into most of an hour.
The release workflow builds each architecture on a runner of that architecture,
stages the two binaries, and points the builder stage at a base with no toolchain
in it -- so the release never pulls a Rust image to run a `cp`.

To stage one by hand, on Linux with `musl-tools` installed:

```bash
CC_x86_64_unknown_linux_musl=musl-gcc \
CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=musl-gcc \
  cargo build --release --target x86_64-unknown-linux-musl -p doppel-cli
mkdir -p dist && cp target/x86_64-unknown-linux-musl/release/doppel dist/
docker build -t doppel:dev .
```

It has to be a musl build. A glibc binary does not run on Alpine, and the failure
is a bare `not found` from the shell rather than anything that says why. On macOS
there is no musl toolchain to install, which is the reason the Dockerfile compiles
inside: `ring` builds C, so a musl target needs a musl C compiler and not only the
Rust standard library for it.

### The dashboard

It is embedded at compile time, so the compile branch needs it: a binary built
without `frontend/dist` starts, serves the API and answers 503 at its own root. The
image therefore builds it when the context has no `frontend/dist`, with the
`nodejs`/`npm` packages installed in the builder stage for exactly that.

```
doppel: no dashboard in frontend/dist, building it
```

Building it on the host first is faster and is what `make image` does -- a working
checkout has it already, and `npm ci` inside a container downloads the whole
dependency tree again. It is not required.

### When it refuses to build

Two refusals, both deliberate:

- **"there is no built dashboard ... and no frontend sources"** -- neither
  `frontend/dist` nor the files `vite build` reads reached the context. That means
  `.dockerignore` was edited, or the build ran from somewhere other than the
  repository root. Stage a binary, or fix the context.
- **"this builder image has no Rust toolchain"** -- something passed
  `--build-arg BUILDER=` naming an image without cargo, and there was no staged
  binary to copy. Stage one, or drop the argument.
