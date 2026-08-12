# Doppel, on Alpine.
#
# Prefers an already-built binary and compiles one only when none was staged.
#
# The preference is not a style choice. Building Rust inside a multi-architecture
# `docker buildx` means QEMU emulation for the non-native architecture, which turns
# a two-minute compile into most of an hour -- so the release workflow builds each
# binary on a native runner and stages it, and this file copies it in.
#
# The fallback exists because a developer should not have to know that. `docker
# build -t doppel:dev .` works with an empty `dist/`: the binary is compiled in the
# builder stage below, for the platform being built, with no cross toolchain on the
# host. `make image` stages one first and this file then skips the compile.
#
#   docker build -t doppel:dev .   # compiles inside, or uses dist/ if it is there
#   make image                     # builds the dashboard first, then this
#
# The dashboard is built inside too, when `frontend/dist` is not in the context. It
# is embedded at compile time, so a binary built without it answers 503 at its own
# root -- and a fresh checkout should not have to know that before it can build an
# image. `make image` still builds it on the host first, which is faster and is what
# a working checkout already has.
#
# A binary you built yourself is used by putting it at `dist/doppel`, or at
# `dist/<platform>/doppel` for a multi-platform build. There is no build argument
# to point elsewhere: one place to look is one thing to get wrong.
#
# A staged binary must be musl-linked. A glibc build does not run here, and the
# failure is a bare "not found" from the shell rather than anything that says why.

# The toolchain from rust-toolchain.toml, so a compile here matches a compile on a
# host. Override with `--build-arg BUILDER=alpine:3.24` when the binary is
# certainly staged: the copy branch needs no toolchain, and the release workflow
# passes exactly that to avoid pulling a Rust image it will not use.
ARG BUILDER=rust:1.94.0-alpine

FROM ${BUILDER} AS builder

# Set by buildx per platform, and the layout the release workflow stages into:
# `dist/linux/amd64/doppel`, `dist/linux/arm64/doppel`. This is what lets one
# Dockerfile pick the right binary out of a multi-platform build without emulating
# anything.
#
# `TARGETOS`/`TARGETARCH` are looked at as well as `TARGETPLATFORM`, because they
# do not always agree: the classic builder reports `linux/arm64/v8` where buildx
# reports `linux/arm64`, and a directory named after the first would be a binary
# nobody finds.
ARG TARGETPLATFORM
ARG TARGETOS
ARG TARGETARCH

WORKDIR /src

# Whatever is in `dist/`, if anything.
#
# `.dockerignore` is always in the context, so this always has one source that
# matches and cannot fail when nothing was staged -- which is the whole point of
# the fallback. Docker has no conditional `COPY`, and this is how "copy it if it is
# there" is expressed. `dist*` rather than a path into it: a glob tolerates a
# missing file but not a missing parent directory, and `dist/linux/arm64/` is
# exactly the parent that does not exist on a laptop.
COPY .dockerignore dist* /staged/

# The workspace, for the branch that compiles. Bounded by `.dockerignore`, which
# admits the crates, the manifests and the built dashboard and nothing else.
COPY . .

# One of two things, decided by what was staged.
#
# The compile branch needs `frontend/dist`, because the admin crate embeds whatever is
# there and an empty directory yields a binary that starts, serves the API, and
# answers 503 at its own root. So it builds the dashboard when nobody has: the
# alternative was refusing, which meant the image could not be built from a fresh
# checkout without knowing to run `make frontend` first.
RUN set -eu; \
    staged=""; \
    for candidate in \
        "/staged/${TARGETPLATFORM}/doppel" \
        "/staged/${TARGETOS}/${TARGETARCH}/doppel" \
        "/staged/doppel"; \
    do \
        if [ -f "$candidate" ]; then staged="$candidate"; break; fi; \
    done; \
    if [ -n "$staged" ]; then \
        echo "doppel: using the binary staged at $staged"; \
        install -m 0755 "$staged" /doppel; \
        exit 0; \
    fi; \
    echo "doppel: nothing staged for ${TARGETPLATFORM}, compiling"; \
    if ! command -v cargo >/dev/null; then \
        echo "doppel: this builder image has no Rust toolchain, and there is no" >&2; \
        echo "        binary in dist/ to copy. Either stage one or drop the" >&2; \
        echo "        BUILDER build argument, whose default carries a toolchain." >&2; \
        exit 1; \
    fi; \
    if [ ! -f frontend/dist/index.html ]; then \
        if [ ! -f frontend/package.json ]; then \
            echo "doppel: there is no built dashboard in frontend/dist and no frontend" >&2; \
            echo "        sources to build one from. Check .dockerignore, or stage a" >&2; \
            echo "        binary in dist/." >&2; \
            exit 1; \
        fi; \
        echo "doppel: no dashboard in frontend/dist, building it"; \
        apk add --no-cache nodejs npm >/dev/null; \
        npm --prefix frontend ci; \
        npm --prefix frontend run build; \
    fi; \
    if [ ! -f frontend/dist/index.html ]; then \
        echo "doppel: the dashboard build produced no frontend/dist/index.html, and a" >&2; \
        echo "        binary embedding nothing answers 503 at its own root." >&2; \
        exit 1; \
    fi; \
    apk add --no-cache musl-dev >/dev/null; \
    cargo build --release --locked -p doppel-cli; \
    install -m 0755 target/release/doppel /doppel

FROM alpine:3.24

# `ca-certificates` is not optional: a proxy whose upstream is https cannot
# verify it without a trust store, and the failure surfaces as UPSTREAM_ERROR
# on every request rather than as a missing package.
#
# `tini` because Doppel drains on SIGTERM. As PID 1 without an init, signals
# from `docker stop` are delivered to a process that has no default handler
# installed for them, and the graceful drain never runs -- the container is
# killed after the timeout instead.
#
# `curl` for the healthcheck. Busybox's wget would do it for nothing, but its
# flags depend on how busybox was compiled and a healthcheck that fails for
# that reason reports the container unhealthy with the cause buried in
# `docker inspect`. curl costs about a megabyte, behaves the same everywhere,
# and is the thing you reach for anyway once you are inside the container
# working out why an upstream is not answering.
#
# Versions are deliberately not pinned here, which is the one thing hadolint
# objects to (DL3018). Alpine's repositories carry only the current build of a
# package, so a pinned version stops resolving within weeks and the image
# stops building for a reason that has nothing to do with this project. The
# base image is pinned to a minor instead, which is what actually fixes the
# package set.
RUN apk add --no-cache ca-certificates curl tini

# Runs as a non-root user. The listeners are configurable, so nothing here
# needs a privileged port; a configuration that asks for one fails at bind,
# which is the right answer in a container.
RUN addgroup -S doppel && adduser -S -G doppel -h /var/lib/doppel doppel

# From the builder stage, which either copied the staged binary or compiled one.
COPY --from=builder /doppel /usr/local/bin/doppel

# Two mount points, both owned by the runtime user.
#
#   /etc/doppel   the configuration. No default is shipped: a configuration
#                 that silently proxied somewhere unintended would be worse
#                 than a container that refuses to start.
#   /var/lib/doppel/templates
#                 template files. A volume because templates uploaded through
#                 the admin API at runtime are written here, and without one
#                 they vanish with the container.
RUN mkdir -p /etc/doppel /var/lib/doppel/templates \
    && chown -R doppel:doppel /var/lib/doppel /etc/doppel

VOLUME ["/var/lib/doppel/templates"]

USER doppel

# Load bearing, not cosmetic. `main.example.yaml` writes
# `templates.dir: ./templates`, a relative path, and this is what makes it
# resolve to the volume mounted above. Moving this line silently sends
# uploaded templates somewhere that is not a volume, and they then vanish with
# the container with nothing reporting it.
WORKDIR /var/lib/doppel

# The proxy and the admin listener. Both are whatever the configuration says;
# these are the ports the example configuration uses and what the
# documentation's `docker run` line publishes.
EXPOSE 8080 8081

# `--config` is given rather than relying on the default `./main.yaml`, so the
# mount point is stated in one place and `docker run` needs no arguments.
ENTRYPOINT ["/sbin/tini", "--", "/usr/local/bin/doppel"]
CMD ["serve", "--config", "/etc/doppel/main.yaml"]
