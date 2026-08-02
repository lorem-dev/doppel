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

## What it does not do yet

Named plainly, because a tool's honest boundary is more useful than its
roadmap:

- No admin HTTP API, so no CRUD over proxies, no Swagger, no Prometheus
  metrics endpoint and no template upload. The configuration file is the only
  way in.
- No PostgreSQL-backed configuration.
- No TCP proxying. A `type: tcp` proxy is rejected at load with a message
  saying so, rather than being quietly ignored.

The configuration model already accepts and validates the settings those
features will use, so a config written today will not need rewriting when they
land.

## The shape of it

```
client  -->  doppel  -->  upstream
              |
              +-- resolve which proxy handles this request (by header, or the default)
              +-- maybe drop it            (loss)
              +-- maybe delay it           (latency)
              +-- maybe answer it itself   (a matching mock, subject to `replace`)
              +-- otherwise forward it
```

That order is fixed. Faults are a property of the proxy, so they apply before
an endpoint is chosen; a mock replaces the endpoint, so it comes after.

## Where to go next

- [Getting started](getting-started.md) -- run it against a real backend in a
  few minutes.
- [Configuration reference](configuration.md) -- every field.
- [Mocks and templating](mocks.md) -- matching, variables, rendering, and the
  sharp edges.
