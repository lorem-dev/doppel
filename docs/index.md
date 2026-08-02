# Doppel

Doppel is a doppelganger for your backend: an HTTP proxy that stands in front
of a real service, or in place of one that is not there.

It forwards traffic upstream, injects controlled faults on the way, and can
replace selected endpoints with templated responses. The point is to let
clients of a backend be developed and tested against something realistic --
including a backend that is slow, that drops requests, or that does not exist
yet.

## What it does

- **Proxies** HTTP to a configured upstream, streaming request and response
  bodies so a transfer larger than memory passes through.
- **Injects faults** -- latency and loss -- as a percentage of requests, per
  proxy and overridable per endpoint.
- **Serves mocks** for matching requests, rendering the response from Jinja2
  templates with variables taken from the path, headers, query and body.
- **Resolves several proxies** behind one port, selected by a request header.
- **Reloads** its configuration without dropping requests in flight.
- **Serves an admin API** -- proxy CRUD, template upload, reload, status,
  Prometheus metrics and Swagger UI -- behind token access control. See
  [Admin API](usage/admin-api.md).
- **Reports to Sentry**, optionally, behind a cargo feature.
- **Stores its configuration** in a YAML file or in PostgreSQL, with
  `config push` and `config pull` to move between them. See
  [Configuration storage](usage/storage.md).

## What it does not do yet

Named plainly, because a tool's honest boundary is more useful than its
roadmap:

- No TCP proxying. A `type: tcp` proxy is rejected at load with a message
  saying so, rather than being quietly ignored.

The configuration model already accepts and validates the settings those
features will use, so a config written today will not need rewriting when they
land.

## Where to go next

The documentation is in three parts.

**Overview** -- this page, and [Concepts](overview/concepts.md), which fixes
the six words the rest of it leans on and shows how a request is handled.

**Usage** -- ordered from first run to the awkward cases:

1. [Getting started](usage/getting-started.md) -- running it against a real
   backend in a few minutes.
2. [Proxying to an upstream](usage/proxying.md) -- URL joining, headers,
   timeouts, and what is refused.
3. [Injecting faults](usage/faults.md) -- latency, loss, and replacing a
   backend gradually.
4. [Mocking endpoints](usage/mocks.md) -- matching, variables, templates, and
   the sharp edges.
5. [Several backends behind one port](usage/multiple-backends.md) -- header
   resolution.
6. [Changing configuration while it runs](usage/runtime-changes.md) -- reloads,
   tokens, and editing one proxy over HTTP.
7. [The admin API](usage/admin-api.md) and
   [PostgreSQL](usage/storage.md) for anything running in more than one place.

with the [configuration reference](usage/configuration.md) and the
[CLI reference](usage/cli.md) at the end for looking things up.

**Development** -- [architecture and the crates](development/architecture.md),
and [how to work on it](development/index.md).
