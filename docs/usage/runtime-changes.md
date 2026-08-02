# Changing configuration while it runs

Doppel does not need restarting to change what it serves. There are three ways
in, and they suit different situations.

| Way in | Reaches | Use it when |
|---|---|---|
| `doppel config reload` | The control socket | You edited the file, or another instance changed the database |
| `POST /api/v1/config/reload` | The admin listener | The same, from something that only has HTTP |
| `POST /api/v1/proxies` and friends | The admin listener | You want to change one proxy without touching a document |

All three end in the same place: a whole new configuration is loaded,
validated and compiled, and only then swapped in. A failure anywhere leaves the
running one untouched, and requests in flight finish against the configuration
they started with.

## Editing the file and reloading

```bash
$EDITOR main.yaml
doppel config reload --socket /tmp/doppel.sock
```

```
reloaded: revision 7a3f2c1908e4bb52, 3 proxies
```

If the edit was bad, nothing changes and every violation is listed:

```
reload rejected: CONFIG_INVALID
proxies[0].latency.min: min must be <= max
admin.access.create: `create` must not be public: ...
```

Some sections cannot take effect without a restart, because their listener is
already bound. Those are reported rather than silently ignored:

```
reloaded: revision 91b0..., 3 proxies
note: these sections changed but only take effect after a restart: server, admin
```

A change to `admin.tokens` or `admin.access` *does* take effect -- what needs a
restart is the host, the port and whether the listener runs at all.

## Issuing a token

The admin API needs a token, and the first one has to come from somewhere. The
control socket is that somewhere: it is reachable by anyone who can read the
socket file, which is the same trust boundary as editing the configuration.

```bash
doppel token add --name ci --group admin
```

```
token `ci` issued to group `admin`, revision 4a19...
it is in force now, and this is the only time it is shown:
3f2504e0-4f89-41d3-9a0c-0305e82c3301
```

The value is generated, written into the stored configuration, and brought into
force before the command answers -- so it works on the next request rather than
after someone remembers to reload.

Capture it without a parser:

```bash
TOKEN=$(doppel token add --name ci --group admin | tail -1)
```

`--group` defaults to `user`, which carries no write access under the default
access block. A token meant for administration needs `--group admin` spelled
out.

## Changing one proxy over HTTP

```bash
curl -s -H "X-Proxy-Authorization: Bearer $TOKEN" \
     localhost:8081/api/v1/proxies
```

```json
{"proxies":[{"revision":"afecb8ebcfbd4471","proxy":{"name":"backend", ...}}]}
```

Each proxy carries its own revision, also returned in `ETag`. Send it back to
make the write conditional:

```bash
curl -s -X PUT \
     -H "X-Proxy-Authorization: Bearer $TOKEN" \
     -H 'If-Match: "afecb8ebcfbd4471"' \
     -H 'Content-Type: application/json' \
     -d '{"name":"backend","type":"http","url":"https://api.example.com/v2/","resolve":{"type":"default"}}' \
     localhost:8081/api/v1/proxies/backend
```

A `409` means someone else changed that proxy since you read it. Read it again,
reapply your change, and retry -- do not strip the header.

The revision is per proxy, so two people editing two different proxies do not
collide.

## Uploading a template at runtime

A mock naming `template: greeting.json.j2` reads that file at request time, so
it can be uploaded to a running process:

```bash
curl -s -X PUT \
     -H "X-Proxy-Authorization: Bearer $TOKEN" \
     --data-binary @greeting.json.j2 \
     localhost:8081/api/v1/proxies/backend/templates/greeting.json.j2
```

No reload is needed -- the next matching request renders the new file.

Only files a proxy's configuration actually names can be uploaded. An upload
for a name no mock declares is refused, so the templates directory cannot
become a place to put arbitrary files.

## Two instances, one database

With `--store postgres`, both instances read the same configuration and agree
on its revision. What they do **not** get is automatic propagation: each keeps
serving what it compiled until told to reload.

```bash
# On the instance making the change
doppel token add --name ci --group admin

# On every other instance
doppel config reload
```

A rolling change is a write followed by a reload on each instance. Nothing
coordinates them, and nothing stops two instances running different revisions
in between; `GET /status` on each reports which one it is holding.

See [Storing configuration in PostgreSQL](storage.md).

## Checking what is running

```bash
curl -s localhost:8081/status
```

```json
{
  "uptime_seconds": 412,
  "revision": "598781ecf030d385",
  "proxies": [
    {"name": "production", "upstream": "http://127.0.0.1:45799/", "resolve": "default", "mocks": 0},
    {"name": "offline", "upstream": "http://127.0.0.1:45799/", "resolve": "header:X-Backend", "mocks": 1}
  ]
}
```

`proxies` is the list, not a count, and each entry says how it is reached --
which is the quickest way to confirm a resolution header is spelled the way you
think it is. Credentials in an upstream URL are redacted here.

`/status` needs no token by default, so it is usable as a health check and as a
way to confirm a rolling reload actually reached every instance.
