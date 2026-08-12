//! The browser dashboard: the page, its assets, and `robots.txt`.
//!
//! The assets are embedded by `build.rs`, which walks `frontend/dist` and writes
//! the table included below. A binary built without that directory carries an
//! empty table and answers 503 here, so a source build on a machine with no Node
//! still works and says why the dashboard is missing.

use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderName, HeaderValue, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use doppel_core::{Error, ErrorCode};
use serde::Serialize;

use crate::response::ApiError;
use crate::state::AdminState;

// `ASSETS` and `INDEX_HTML`, generated at build time.
include!(concat!(env!("OUT_DIR"), "/assets.rs"));

/// The placeholder `index.html` carries, replaced per request.
///
/// Matched as a whole element rather than by editing its contents: a partial
/// match that silently failed would leave the development placeholder in a
/// served page, and the page would then report the wrong title, the wrong
/// version, and `public: false` for a public deployment.
const CONFIG_ELEMENT_ID: &str = "doppel-config";

/// Kept out of a search index three ways, because they fail differently: a
/// crawler that reads headers sees this, one that reads markup sees the `meta`
/// element in `index.html`, and one that reads `robots.txt` sees the route
/// below.
const ROBOTS_TAG: (HeaderName, HeaderValue) = (
    HeaderName::from_static("x-robots-tag"),
    HeaderValue::from_static("noindex, nofollow, noarchive"),
);

/// Sent with everything here.
///
/// `nosniff` matters most on the asset route: without it a browser may decide a
/// file is something other than what its `Content-Type` says, and the whole
/// point of serving hashed assets with an exact type is that it does not have to
/// guess.
const NO_SNIFF: (HeaderName, HeaderValue) = (
    HeaderName::from_static("x-content-type-options"),
    HeaderValue::from_static("nosniff"),
);

/// No inline script, no inline style, no framing, nothing fetched from anywhere
/// but this origin.
///
/// The configuration reaches the page as the *contents* of a
/// `application/json` element rather than as inline JavaScript, which is what
/// makes `script-src 'self'` sufficient -- an inline `<script>window.config = ...`
/// would have forced `unsafe-inline` and given up most of this.
const CSP: (HeaderName, HeaderValue) = (
    HeaderName::from_static("content-security-policy"),
    HeaderValue::from_static(
        "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; \
         connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
    ),
);

/// Whether this binary carries the dashboard's static assets.
///
/// False for a build that had no `frontend/dist`. The tests that exercise real
/// asset delivery consult this rather than a cfg, and CI turns a skip into a
/// failure through `DOPPEL_REQUIRE_DASHBOARD_ASSETS` -- the same mechanism the
/// database suites use, for the same reason: a gate that silently skips is worse
/// than no gate.
#[must_use]
pub fn is_built() -> bool {
    !INDEX_HTML.is_empty()
}

pub fn routes() -> Router<AdminState> {
    Router::new()
        .route("/", get(index))
        .route("/static/{*path}", get(asset))
        .route("/robots.txt", get(robots))
}

/// A path the dashboard's router is allowed to answer with the page.
///
/// Everything the API serves lives under `/api/`, so everything outside it belongs
/// to the dashboard -- and a client-side route reached by a reload rather than by a
/// link has to arrive at the page rather than at a 404. `/status` was the case that
/// showed this up: the dashboard has a page at that path and the API used to have
/// an endpoint there, and a reload got the endpoint's JSON.
///
/// `/static/` is deliberately excluded. A missing asset must stay a 404: answering
/// it with the page means a typo'd script tag loads HTML, which fails later and
/// somewhere else.
fn belongs_to_the_page(path: &str) -> bool {
    !path.starts_with("/api/") && !path.starts_with("/static/")
}

/// The dashboard's answer for a path nothing else claimed.
///
/// A GET the page could route is the page; anything else keeps the error envelope,
/// so a mistyped API path and a wrong method still answer as the API.
pub async fn fallback(
    state: State<AdminState>,
    method: axum::http::Method,
    uri: axum::http::Uri,
) -> Response {
    if method == axum::http::Method::GET && belongs_to_the_page(uri.path()) {
        return index(state).await;
    }
    refuse(Error::new(
        ErrorCode::NotFound,
        format!("no route for {method} {}", uri.path()),
    ))
}

/// What the page is told about the process serving it.
///
/// Rendered into the HTML rather than fetched: every value is known when the
/// HTML is written, and a second request would mean the page cannot draw its own
/// header until a round trip finishes.
#[derive(Debug, Serialize)]
struct PageConfig<'a> {
    title: &'a str,
    /// Whether the admin API is unauthenticated. `true` and the page never asks
    /// for a token.
    public: bool,
    version: &'a str,
    /// `admin.auth.header`, which is configurable -- so the page has to be told
    /// rather than assume the default.
    #[serde(rename = "authHeader")]
    auth_header: &'a str,
    /// How often the proxy list is refetched.
    #[serde(rename = "refreshMs")]
    refresh_ms: u64,
}

/// The proxy list is refetched once a minute.
///
/// Sent to the page rather than written there, so the interval is one value in
/// one place.
const REFRESH_MS: u64 = 60_000;

async fn index(State(state): State<AdminState>) -> Response {
    if !is_built() {
        return refuse(Error::new(
            ErrorCode::DashboardNotBuilt,
            "this binary was built without the dashboard's static assets; build them with \
             `npm --prefix frontend ci && npm --prefix frontend run build` and rebuild, or set \
             `admin.dashboard: false` to stop serving this route",
        ));
    }

    // The reloaded configuration, not the startup one: a reload that changes
    // `admin.title` should change the page's heading, and `public` decides
    // whether the page asks for a token at all.
    let config = crate::access::policy(&state);
    let page = PageConfig {
        title: config.admin.title(),
        public: config.admin.is_public(),
        version: env!("CARGO_PKG_VERSION"),
        auth_header: config.admin.auth.header.as_str(),
        refresh_ms: REFRESH_MS,
    };

    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/html; charset=utf-8"),
            ),
            // Never cached. The document carries the configuration, so a cached
            // copy would keep showing a title, a version and a `public` flag
            // from before the last reload.
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
        [ROBOTS_TAG, NO_SNIFF, CSP],
        render(INDEX_HTML, &page),
    )
        .into_response()
}

/// Substitute the configuration into the placeholder element.
///
/// Every less-than character in the JSON is replaced by its JSON unicode
/// escape. Inside an `application/json` script element the only sequence that
/// can end the element early is a closing script tag, so removing the less-than
/// removes the whole class of injection -- which is what makes a hostile
/// `admin.title` a non-event rather than a scripting hole.
fn render(template: &str, page: &PageConfig<'_>) -> String {
    let json = serde_json::to_string(page)
        .expect("PageConfig is plain data")
        .replace('<', "\\u003c");

    // `build.rs` refuses to build a binary whose `index.html` lacks the element,
    // so this cannot fail at request time. It used to return the page with a
    // message in it instead -- an untestable branch guarding an invariant that
    // belongs to the build, and the build is where it is now checked.
    let (before, rest) =
        split_at_element(template).expect("build.rs checks index.html carries the element");
    format!("{before}{json}{rest}")
}

/// Split the template around the placeholder's contents.
///
/// Returns the text up to and including the opening tag, and the text from the
/// closing tag onwards.
fn split_at_element(template: &str) -> Option<(&str, &str)> {
    let id = format!("id=\"{CONFIG_ELEMENT_ID}\"");
    let at = template.find(&id)?;
    let open_ends = template[at..].find('>')? + at + 1;
    let close_begins = template[open_ends..].find("</script>")? + open_ends;
    Some((&template[..open_ends], &template[close_begins..]))
}

async fn asset(Path(path): Path<String>) -> Response {
    let Some((_, bytes, content_type)) = ASSETS.iter().find(|(name, _, _)| *name == path) else {
        // The envelope, not the page. A mistyped API path must not answer 200
        // with an HTML document: a client would parse it as a successful
        // response, and a human would see a working dashboard where they asked
        // for something that is not there.
        return refuse(Error::new(
            ErrorCode::NotFound,
            format!("no asset /static/{path}"),
        ));
    };

    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(content_type)),
            // Safe because vite content-hashes every filename: a changed file is
            // a different URL, so nothing has to expire.
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=31536000, immutable"),
            ),
        ],
        [ROBOTS_TAG, NO_SNIFF],
        *bytes,
    )
        .into_response()
}

/// An error from a dashboard route, carrying the same headers its successes do.
///
/// The robots header belongs on a refusal as much as on a page: a crawler that
/// reached `/static/typo.js` would otherwise be free to index the error body, and
/// "every dashboard response forbids indexing" is easier to hold to than a list
/// of the ones that do.
fn refuse(error: Error) -> Response {
    let mut response = ApiError::from(error).into_response();
    response.headers_mut().extend([ROBOTS_TAG, NO_SNIFF]);
    response
}

async fn robots() -> Response {
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        )],
        [ROBOTS_TAG, NO_SNIFF],
        "User-agent: *\nDisallow: /\n",
    )
        .into_response()
}
