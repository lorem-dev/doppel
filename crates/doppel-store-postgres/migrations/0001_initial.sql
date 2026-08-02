-- The configuration schema.
--
-- Minimal normalisation: anything with its own identity and lifecycle is a
-- row, and a leaf map hanging off one is a JSONB column on that row. One
-- opaque blob would throw away the per-proxy concurrency the admin API is
-- built on; a table per leaf map would buy joins nobody will write.

CREATE TABLE configurations (
    name               TEXT PRIMARY KEY,
    -- The content-derived revision, not a counter: two instances holding the
    -- same configuration compute the same value, which is what makes the
    -- compare-and-swap below meaningful across processes. It is a signed
    -- BIGINT because PostgreSQL has no unsigned integers; the u64 is stored
    -- through a documented bit-for-bit cast, not a numeric conversion.
    revision           BIGINT NOT NULL,
    server_host        TEXT NOT NULL,
    server_port        INTEGER NOT NULL,
    log_level          TEXT NOT NULL,
    log_format         TEXT NOT NULL,
    control_socket     TEXT NOT NULL,
    templates_dir      TEXT NOT NULL,
    sentry_dsn         TEXT,
    -- Whether the admin listener runs at all. Defaulted to true, matching the
    -- configuration default, so a hand-written INSERT that omits it means the
    -- same thing in the database as an omitted field means in the document.
    admin_enable       BOOLEAN NOT NULL DEFAULT true,
    admin_host         TEXT NOT NULL,
    admin_port         INTEGER NOT NULL,
    admin_auth_header  TEXT NOT NULL,
    admin_upload_limit BIGINT NOT NULL,
    admin_access       JSONB NOT NULL,
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE admin_tokens (
    config  TEXT NOT NULL REFERENCES configurations ON DELETE CASCADE,
    name    TEXT NOT NULL,
    "group" TEXT NOT NULL,
    token   TEXT NOT NULL,
    ordinal INTEGER NOT NULL,
    PRIMARY KEY (config, name)
);

-- Token values are unique within a configuration, which is the same
-- constraint validation applies to the document. Expressing it here too means
-- a hand-written INSERT cannot produce a configuration the loader would
-- refuse.
CREATE UNIQUE INDEX admin_tokens_value ON admin_tokens (config, token);

CREATE TABLE proxies (
    config             TEXT NOT NULL REFERENCES configurations ON DELETE CASCADE,
    name               TEXT NOT NULL,
    -- Document order. Not decoration: mock patterns are unanchored, so a
    -- general one placed before a specific one shadows it, and a store that
    -- reordered them would change which mock answers a request.
    ordinal            INTEGER NOT NULL,
    kind               TEXT NOT NULL,
    url                TEXT NOT NULL,
    -- Seconds, and a whole number: `ProxyConfig::timeout` is `Option<u64>`.
    -- A floating-point column would round-trip a value the type cannot hold.
    timeout_seconds    BIGINT,
    body_limit         BIGINT NOT NULL,
    -- Nullable, because the field is `Option<f64>`: absent means "use the
    -- default", which is not the same as 1.0 written out.
    replace_ratio      DOUBLE PRECISION,
    resolve_kind       TEXT NOT NULL,
    resolve_header     TEXT,
    loss_percentage    DOUBLE PRECISION,
    loss_status        INTEGER,
    latency_percentage DOUBLE PRECISION,
    latency_min        DOUBLE PRECISION,
    latency_max        DOUBLE PRECISION,
    headers            JSONB NOT NULL,
    access             JSONB,
    PRIMARY KEY (config, name)
);

CREATE TABLE mocks (
    config           TEXT NOT NULL,
    proxy            TEXT NOT NULL,
    name             TEXT NOT NULL,
    ordinal          INTEGER NOT NULL,
    method           TEXT NOT NULL,
    url_pattern      TEXT NOT NULL,
    status           INTEGER NOT NULL,
    body             TEXT,
    json             TEXT,
    template         TEXT,
    request_headers  JSONB NOT NULL,
    request_query    JSONB NOT NULL,
    request_body     JSONB NOT NULL,
    response_headers JSONB NOT NULL,
    proxy_override   JSONB,
    PRIMARY KEY (config, proxy, name),
    FOREIGN KEY (config, proxy) REFERENCES proxies (config, name) ON DELETE CASCADE
);

-- Deliberately no foreign key to `proxies`. Deleting a proxy must delete its
-- templates, and the store does that explicitly, after the configuration
-- write and never before -- the write is what authorises dropping the files.
-- A cascade would do it silently and move that ordering decision into the
-- schema, where the reasoning behind it is invisible.
CREATE TABLE templates (
    config  TEXT NOT NULL,
    proxy   TEXT NOT NULL,
    file    TEXT NOT NULL,
    content BYTEA NOT NULL,
    PRIMARY KEY (config, proxy, file)
);
