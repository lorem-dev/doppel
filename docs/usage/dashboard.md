# The dashboard

The admin listener serves a browser dashboard from its own root. It lists the
proxies, edits them, writes mock templates, shows what the process is doing, and
reloads the configuration -- everything the admin API does, without curl.

It is on by default. Open the admin port in a browser:

```
http://127.0.0.1:8081/
```

The whole thing is compiled into the binary. There is nothing to deploy, no
directory to serve, and no version of the page that can disagree with the version
of Doppel serving it.

Every URL the dashboard shows is a real one. `/proxies/alpha` can be bookmarked,
reloaded and shared, because everything outside `/api/` and `/static/` is answered
with the page -- which is also why the API lives under `/api/` in the first place:
`/status` used to be an endpoint *and* a page, and a reload got the JSON.

---

## Turning it on and off

```yaml
admin:
  dashboard: true      # the default
  title: "Doppel"      # the default
```

`dashboard: false` leaves `/`, `/static/*` and `/robots.txt` unrouted -- they
answer 404 like any other unknown path -- and changes nothing about the JSON API.
Use it when the admin port is reachable from somewhere a browser page has no
business being. To turn off the admin API entirely, that is `admin.enable`, not
this.

Both fields take effect on restart, because the routes are built once at startup.

`title` is what the header shows and what the browser tab is called. It is worth
setting when several Doppels are open at once:

```yaml
admin:
  title: "billing-api (staging)"
```

At most 64 characters, no control characters, and any language you like -- it is a
heading, not an identifier.

---

## Who can use it

The dashboard is a client of the admin API and is bound by exactly the same
`access` rules. It has no privileges of its own, and there is no dashboard
password: the token is the one from `admin.tokens`.

What follows from that is worth stating plainly, because it surprises people:

**The page works without a token.** If `access.list` and `access.read` are
`public`, an anonymous visitor sees the proxy list and can open a proxy. The token
dialog opens once, and it has a "Continue without a token" button -- declining
leaves a working page rather than a wall.

**Actions the caller may not perform are visible but disabled**, with the reason
in the tooltip. The page asks the server what the caller may do
(`GET /api/v1/access`) rather than guessing, so a disabled button means the server
would have refused. A missing button reads as a broken page; a disabled one
explains itself.

**Signing out returns to the anonymous view.** It does not lock the page.

### The token in the browser

Entering a token stores it in `localStorage` for one hour, then the page forgets
it and asks again. Two things that is not:

- **Not a session.** The token does not expire on the server. Forgetting it here
  only means this browser stops presenting it. Revoking a token is a
  configuration change -- edit `admin.tokens` and reload.
- **Not a security boundary.** Anything running in the page can read
  `localStorage`. The hour exists so an unattended browser stops holding a working
  admin token indefinitely, which is a smaller and more honest claim.

On a `public: true` deployment the page never mentions tokens at all.

---

## What the pages do

**Proxies** lists every proxy with its upstream, how it is resolved, the faults
configured on it and how many mocks it has. The list refetches itself once a
minute -- so a change made by another operator, by `doppel config push`, or by a
reload appears without anyone pressing anything. It pauses while the tab is
hidden.

Editing opens a form over the whole proxy document: the upstream, timeouts, body
limits, resolution, injected headers, the three fault settings, per-proxy access
overrides, and the mocks with their matching rules and responses.

A proxy has thirteen fields and any number of mocks, and most edits touch one of
them -- so the name, the type and the upstream are always on screen and everything
else is a folded section that says what is inside it: `Faults none`,
`Access overrides inherited`, `Mocks 3`. Save and Cancel sit in a bar pinned to the
bottom of the window, so they are reachable whatever is open.

Saving sends the revision the form was loaded at, so a proxy someone else changed
in the meantime is refused rather than overwritten -- the page says so and asks you
to reload.

**Templates**, per proxy, lists the template files and lets you write one: a name,
one of `json.j2`, `html.j2` or `text.j2`, and the content, with syntax colouring.
There is no file upload; the content is typed or pasted, which is what the API has
always taken.

One rule the server enforces and the page cannot: a template file has to be
declared by one of the proxy's mocks before it may be written. Add the mock first,
then the file.

**Status** shows uptime, the configuration revision in effect and the resolution
state of each proxy, and carries the reload button. Reload re-reads the store and
reports what it applied, including the sections that need a restart instead.

**API** is not a page of the dashboard but a link to the Swagger UI the same
listener serves, at `/api/swagger-ui/`. It opens in a new tab, because it is a
different application and going there should not throw away a half-filled form.

---

## Theme

Light, dark, or whatever the operating system says. The default is the last of
those, and the choice is remembered per browser.

---

## Not indexed

The dashboard refuses search engines three ways, because crawlers fail
differently: an `X-Robots-Tag: noindex, nofollow, noarchive` header on every
response including the errors, a `robots` meta element in the page for a crawler
that reads markup and ignores headers, and `robots.txt` disallowing everything.

The page also carries a content security policy that permits scripts, styles and
connections from this origin only, and forbids framing. It needs no
`unsafe-inline`: the configuration reaches the page as the contents of a JSON
element rather than as an inline script.

---

## When the root answers 503

```json
{
  "status": "error",
  "code": "DASHBOARD_NOT_BUILT",
  "message": "this binary was built without the dashboard's static assets ..."
}
```

The binary was compiled without `frontend/dist`. Released binaries and the
published image always carry the dashboard -- the release refuses to publish one
that does not -- so this is a binary built from source without building the
frontend first:

```bash
npm --prefix frontend ci
npm --prefix frontend run build
cargo build --release -p doppel-cli
```

That order matters: the assets are embedded at compile time, so rebuilding them
without rebuilding the binary changes nothing about what it serves.

If Node is not available and the JSON API is enough, `admin.dashboard: false`
stops the route being served at all, which is a clearer answer than a 503.
