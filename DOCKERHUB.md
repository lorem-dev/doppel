<!-- The Docker Hub overview for `loremdev/doppel`, pushed by the `describe`
     job in .github/workflows/release.yml.

     Not the README. Docker Hub's page is reached by someone who has already
     chosen the image, so it opens with `docker run` rather than with the
     installer, and it drops everything about building from source. Every link
     is absolute: nothing here is served from a repository checkout.

     Docker Hub caps the overview at 25000 bytes and the short description at
     100, and truncates silently past that. Keep both well clear.

     Every `loremdev/doppel:<version>-alpine` below is rewritten to the tag
     being released before the file is pushed, so the copy-paste commands name
     a version that exists. The version checked in is a placeholder -- `1.2.3`,
     the same shape the tags table uses -- because a real one here would be a
     second version to remember to bump, and the only reader who ever sees this
     file unrewritten is someone with the repository open. -->

![Doppel](https://raw.githubusercontent.com/lorem-dev/doppel/main/assets/icon-128.png)

# Doppel

A CLI-driven HTTP reverse proxy. The name is from "doppelganger": it stands in
front of, or in place of, a real backend, so clients of that backend can be
developed and tested against a realistic, deliberately degraded, or entirely
absent upstream. It forwards traffic to a configured upstream, injects
controlled latency and loss, and replaces selected endpoints with templated
mock responses.

- **Documentation:** <https://lorem-dev.github.io/doppel/>
- **Source:** <https://github.com/lorem-dev/doppel>
- **Licence:** Apache-2.0

## Tags

Published for `linux/amd64` and `linux/arm64`.

| Tag | Moves? | Points at |
|---|---|---|
| `1.2.3-alpine` | no | exactly that release |
| `1.2-alpine` | yes | the newest patch of 1.2 |
| `1-alpine` | yes | the newest minor of 1 |

Every tag names its base, the convention the images this one sits beside
use (`postgres:17.6-alpine`, `node:22-alpine`). That leaves the unsuffixed
names free, so a second base could be published later without renaming
anything anyone has already pinned.

There is deliberately **no `latest`**. A tag that moves is one an unpinned
deployment follows into a release nobody reviewed. Pin the full version in
anything that matters.

A pre-release (`1.2.3-rc.1`) is published as `1.2.3-rc.1-alpine` only. It never
takes `1.2-alpine` or `1-alpine` with it.

## Quick start

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

`8080` carries proxied traffic, `8081` the admin API.

A minimal `config/main.yaml`:

```yaml
server:
  host: "0.0.0.0"
  port: 8080

admin:
  host: "0.0.0.0"
  port: 8081
  tokens: []
  access: {}
  upload:
    limit: 1Mi

proxies:
  - name: backend
    type: http
    url: "https://example.com/"
    resolve:
      type: default
```

Both listeners must bind `0.0.0.0` rather than `127.0.0.1`, or nothing outside
the container can reach them.

## The templates volume is not optional

Templates uploaded through the admin API are written to disk at upload and read
from disk per request. Without a volume they live in the container's writable
layer and go with it -- `docker stop`, `docker run` again, and every uploaded
template is gone. Nothing reports this: the mock that named the file answers
with a render error on the next request, which reads like a broken template
rather than a missing volume.

```yaml
templates:
  dir: /var/lib/doppel/templates
```

A relative `./templates` also works: the image's working directory is
`/var/lib/doppel`, so it resolves to the same place.

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
      # Only needed when the published port differs from the one Doppel binds;
      # these publish the same number. See "Where clients reach it" below.
      # DOPPEL_EXTERNAL_URL: "http://127.0.0.1:8080/"
    healthcheck:
      test: ["CMD", "curl", "-fsS", "-o", "/dev/null", "http://127.0.0.1:8081/api/v1/status"]
      interval: 5s
      timeout: 3s
      start_period: 5s
      retries: 5

volumes:
  doppel-templates:
```

`/api/v1/status` needs no token by default and answers only once the runtime is
compiled and both listeners are bound. `-f` matters: without it `curl` exits 0
on a 500 and the check passes for answering at all rather than for answering
correctly.

## The dashboard

The admin port serves a browser dashboard from its root -- the proxy set, a form
over every field of a proxy including its mocks, status and reload -- compiled
into the image. Publish `8081` as the quick start above does and open
`http://127.0.0.1:8081/`.

Template files are the admin API's, not the page's: it shows which file a mock
uses and will not edit it.

It is a client of the admin API and bound by the same token rules, so it offers
only what the caller's token may actually do. `admin.dashboard: false` turns it
off; the JSON API is unaffected either way.

## Where clients reach it

Doppel rewrites an upstream's redirects, and the addresses in the bodies it
relays, to point back at itself. To do that it needs the address a client used,
which it cannot see: it binds `8080` inside the container, and a port mapping is
invisible from in there.

Publishing the same number, as the quick start does, needs nothing. Publishing a
different one needs telling:

```bash
docker run --rm -p 58080:8080 -p 58081:8081 \
  -e DOPPEL_EXTERNAL_URL=http://127.0.0.1:58080/ \
  ...
```

Without it a rewritten redirect names port 8080, which is not published, and the
client follows it nowhere. Doppel logs the address it settled on at startup.

## Tokens

`DOPPEL_ADMIN_TOKENS` keeps secrets out of the configuration file that gets
mounted in:

```bash
docker run --rm \
  -e DOPPEL_ADMIN_TOKENS='{"ci":{"token":"...","group":"admin"}}' \
  ...
```

## Reloading

`doppel config reload` talks to a control socket inside the container. Two ways
in:

```bash
# Through the admin API, if it is enabled and you hold a token.
curl -X POST -H "X-Proxy-Authorization: Bearer $TOKEN" \
     http://localhost:8081/api/v1/config/reload

# Or the same binary, inside the container.
docker exec <container> doppel config reload --socket /tmp/doppel.sock
```

The directory is mounted rather than the file on purpose: a save renames a new
file over the old one, and a file that is itself a mount point cannot be replaced --
the admin API would refuse every write with "Resource busy".

Editing `config/main.yaml` on the host changes the file the container
reads, so a reload picks it up without restarting anything.

## What the image contains

Alpine, `ca-certificates`, `tini`, and one statically linked binary. It runs as
a non-root user, so a configuration asking for a port below 1024 fails at
bind -- the right answer in a container; publish a low port on the host
instead.

`ca-certificates` is not optional: without a trust store a proxy cannot verify
an `https` upstream, and every request fails with `UPSTREAM_ERROR`.

Doppel runs under `tini`, so `docker stop` delivers `SIGTERM` and the graceful
drain runs -- in-flight requests finish, then the process exits.

## More

Full documentation, including fault injection, mocks, the admin API, PostgreSQL
storage and observability, is at <https://lorem-dev.github.io/doppel/>. The
Docker page there covers this ground in more detail:
<https://lorem-dev.github.io/doppel/usage/docker/>.
