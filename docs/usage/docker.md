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
  -v "$PWD/main.yaml:/etc/doppel/main.yaml:ro" \
  -v doppel-templates:/var/lib/doppel/templates \
  loremdev/doppel:1.2.3-alpine
```

Two mounts, and the second is not optional if you use templates at all --
see [Templates](#templates) below.

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
  -v "$PWD/main.yaml:/etc/doppel/main.yaml:ro" \
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

Editing a bind-mounted `main.yaml` on the host changes the file the container
reads, so a reload picks it up without restarting anything.

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
      - ./main.yaml:/etc/doppel/main.yaml:ro
      - doppel-templates:/var/lib/doppel/templates
    environment:
      DOPPEL_ADMIN_TOKENS: '{"ci":{"token":"...","group":"admin"}}'

volumes:
  doppel-templates:
```

## What the image contains

Alpine, `ca-certificates`, `tini`, and one statically linked binary. It runs as
a non-root user, so a configuration asking for a port below 1024 fails at
bind -- which is the right answer in a container; publish a low port on the
host instead.

`ca-certificates` is not optional: without a trust store a proxy cannot verify
an `https` upstream, and every request fails with `UPSTREAM_ERROR`.

## Building it yourself

The binary is built outside the Dockerfile and copied in. Building Rust inside
a multi-architecture `buildx` means QEMU for the non-native architecture, which
turns a two-minute compile into most of an hour.

```bash
cargo zigbuild --release --target x86_64-unknown-linux-musl -p doppel-cli
mkdir -p dist && cp target/x86_64-unknown-linux-musl/release/doppel dist/
docker build --build-arg BIN=dist/doppel -t doppel:dev .
```

It has to be a musl build. A glibc binary does not run on Alpine, and the
failure is a bare `not found` from the shell rather than anything that says
why.
