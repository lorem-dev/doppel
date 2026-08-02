# Doppel, on Alpine.
#
# The binary is built outside this file and copied in -- see the `docker` job
# in .github/workflows/release.yml. Building Rust inside a multi-architecture
# `docker buildx` means QEMU emulation for the non-native architecture, which
# turns a two-minute compile into most of an hour. Copying an already-built
# binary makes the image build the same speed on both.
#
# Build locally against a binary you already have:
#
#   cargo zigbuild --release --target x86_64-unknown-linux-musl -p doppel-cli
#   mkdir -p dist && cp target/x86_64-unknown-linux-musl/release/doppel dist/
#   docker build --build-arg BIN=dist/doppel -t doppel:dev .
#
# The binary must be musl-linked. A glibc build does not run here, and the
# failure is a bare "not found" from the shell rather than anything that says
# why.

FROM alpine:3.24

# `TARGETPLATFORM` is set by buildx per platform (`linux/amd64`,
# `linux/arm64`), so one Dockerfile picks the right binary out of a directory
# laid out by platform without emulating anything.
#
# `BIN` defaults from it and can be overridden for a build by hand, where
# there is one binary and no buildx:
#
#   docker build --build-arg BIN=dist/doppel -t doppel:dev .
ARG TARGETPLATFORM
ARG BIN=dist/${TARGETPLATFORM}/doppel

# `ca-certificates` is not optional: a proxy whose upstream is https cannot
# verify it without a trust store, and the failure surfaces as UPSTREAM_ERROR
# on every request rather than as a missing package.
#
# `tini` because Doppel drains on SIGTERM. As PID 1 without an init, signals
# from `docker stop` are delivered to a process that has no default handler
# installed for them, and the graceful drain never runs -- the container is
# killed after the timeout instead.
RUN apk add --no-cache ca-certificates tini

# Runs as a non-root user. The listeners are configurable, so nothing here
# needs a privileged port; a configuration that asks for one fails at bind,
# which is the right answer in a container.
RUN addgroup -S doppel && adduser -S -G doppel -h /var/lib/doppel doppel

COPY --chmod=0755 ${BIN} /usr/local/bin/doppel

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
