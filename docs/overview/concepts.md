# Concepts

Six words carry most of the meaning in this documentation. They are worth
fixing before the examples start, because several of them mean something
narrower here than in general use.

## Proxy

A **proxy** is one upstream and everything Doppel does on the way to it: which
requests it handles, what it injects, what it answers itself. It is not a
process and not a port -- several proxies share one listener.

A configuration is a list of them.

## Resolution

**Resolution** is choosing which proxy handles a request. Exactly one proxy may
declare `resolve: {type: default}`; the others are selected by a header naming
them. A request that matches no header goes to the default, and if there is no
default it is refused with `404` and `PROXY_NOT_RESOLVED`.

See [Several backends behind one port](../usage/multiple-backends.md).

## Fault

A **fault** is a deliberate degradation: `loss` drops a share of requests with
a chosen status, `latency` delays a share of them by a random time in a range.
Both are properties of the proxy, so they apply before Doppel decides what will
answer the request.

Shares are written as fractions. `0.1` is one request in ten; `50` is not fifty
percent, it is refused.

See [Injecting faults](../usage/faults.md).

## Mock

A **mock** answers a matching request instead of forwarding it. It matches on
method and on a regular expression against the path, and it renders its
response from a Jinja2 template with variables taken from the path, headers,
query string and body.

Anything that does not match still reaches the upstream, so a configuration can
replace one endpoint of a real backend and leave the rest alone.

See [Mocking endpoints](../usage/mocks.md).

## Revision

A **revision** is a fingerprint of a configuration's content, not a counter. The
same document always produces the same revision, on any machine, in any
process.

That is what makes the admin API's compare-and-swap work: a client sends the
revision it read, and a write whose revision has moved is refused rather than
silently overwriting someone else's edit.

## Store

A **store** is where a configuration lives -- a YAML file or a PostgreSQL
database. Everything above it is written against one trait, so the choice
changes where the configuration is kept and nothing about what the proxy does
with it.

See [Storing configuration in PostgreSQL](../usage/storage.md).

## How a request is handled

The order is fixed:

```
client  -->  doppel  -->  upstream
              |
              |  1. resolve which proxy handles this request
              |  2. maybe answer it here       (a matching mock, subject to `replace`)
              |  3. maybe drop it              (loss)
              |  4. maybe delay it             (latency)
              |  5. otherwise forward it
```

Step 2 is conditional twice over. A mock has to match, and then `replace` --
itself a fraction -- has to fire. `replace: 0.5` sends half of the matching
requests to the real upstream and answers the other half locally, which is how
a backend is replaced incrementally rather than all at once.

Mock matching comes before the faults, and that ordering is what makes
`replace` mean what it says. `loss` and `latency` describe the real backend; a
mock replaces the real backend, so neither applies to a request a mock
answered. Were it the other way round, `replace: 0.5` under `loss: 0.5` would
answer a quarter of matching requests from the mock rather than half, and no
configuration could ask for half while any loss was set.

The consequence worth knowing: a request a mock answers is never dropped and
never delayed, however the proxy's faults are set. The faults are on the path
to the upstream, and a mocked request does not take it.

## Two things that are not what they sound like

**`percentage` is a fraction.** The field name is older than the decision to
use fractions and is kept for compatibility. Every one of them is `0.0` to
`1.0`.

**A `revision` is not a version number.** It does not increase. Two edits that
cancel out return to the revision they started from, which is correct: the
configuration really is the one it was before.
