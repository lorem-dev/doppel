-- Store the configuration as JSON instead of a column per field.
--
-- Why: 0002, 0003 and 0004 each added one column and nothing else. Their whole
-- content was `ADD COLUMN`, plus a matching edit in load.rs, in save.rs, and in
-- two hand-written statements a test existed solely to keep in step. Nothing
-- ever queried `admin_host` or `latency_min` in SQL. A column per field bought
-- nothing and charged a migration for every field the configuration format
-- gained.
--
-- What stays a column is what SQL uses: `name` and `revision` for the
-- compare-and-swap, `ordinal` for proxy order, `config` for the foreign key.
--
-- The backfill below has one job, and it is not cosmetic: the revision of a
-- stored configuration is a hash of its canonical YAML, and every write
-- compare-and-swaps against it. If the JSON these statements build parsed back
-- into a configuration that differed in any way from the one the old columns
-- described, the revision would change, and the next write from a client holding
-- the old revision would be refused as a conflict that never happened. So the
-- reconstruction has to be exact, including the difference between a key that is
-- absent and one that is present and null -- `jsonb_strip_nulls` is what removes
-- the latter, and the sections that are wholly optional (`sentry`, `loss`,
-- `latency`) are built conditionally rather than stripped, because an empty
-- object is not the same as a missing one for a struct with required fields.
--
-- One property is given up here: the unique index on (config, token) that made
-- duplicate token values impossible at the database level. Rule V26 refuses them
-- while the document is validated, which every write goes through, so the
-- guarantee moves rather than disappears.

-- ---------------------------------------------------------------------------
-- configurations
-- ---------------------------------------------------------------------------

ALTER TABLE configurations ADD COLUMN settings JSONB;

UPDATE configurations c
SET settings = jsonb_strip_nulls(
        jsonb_build_object(
            'server', jsonb_build_object(
                'host', c.server_host,
                'port', c.server_port
            ),
            'logging', jsonb_build_object(
                'level', c.log_level,
                'format', c.log_format
            ),
            'control', jsonb_build_object('socket', c.control_socket),
            'templates', jsonb_build_object('dir', c.templates_dir),
            'admin', jsonb_strip_nulls(
                jsonb_build_object(
                    'enable', c.admin_enable,
                    'host', c.admin_host,
                    'port', c.admin_port,
                    'auth', jsonb_build_object('header', c.admin_auth_header),
                    'upload', jsonb_build_object('limit', c.admin_upload_limit),
                    'access', c.admin_access,
                    'public', c.admin_public,
                    'groups', c.admin_groups
                )
            ) || jsonb_build_object(
                -- Built separately because `||` is how a key is added to an
                -- already-stripped object without stripping this one: an empty
                -- token list must survive as `[]`.
                'tokens', COALESCE(
                    (
                        SELECT jsonb_agg(
                                   jsonb_build_object(
                                       'name', t.name,
                                       'group', t."group",
                                       'token', t.token
                                   )
                                   ORDER BY t.ordinal
                               )
                        FROM admin_tokens t
                        WHERE t.config = c.name
                    ),
                    '[]'::jsonb
                )
            )
        )
        -- `sentry` is an optional section with a required `dsn`, so it is either
        -- a whole object or absent. Stripping nulls from `{"dsn": null}` would
        -- leave `{}`, which fails to parse.
        || CASE
               WHEN c.sentry_dsn IS NULL THEN '{}'::jsonb
               ELSE jsonb_build_object('sentry', jsonb_build_object('dsn', c.sentry_dsn))
           END
    );

ALTER TABLE configurations ALTER COLUMN settings SET NOT NULL;

ALTER TABLE configurations
    DROP COLUMN server_host,
    DROP COLUMN server_port,
    DROP COLUMN log_level,
    DROP COLUMN log_format,
    DROP COLUMN control_socket,
    DROP COLUMN templates_dir,
    DROP COLUMN sentry_dsn,
    DROP COLUMN admin_enable,
    DROP COLUMN admin_host,
    DROP COLUMN admin_port,
    DROP COLUMN admin_auth_header,
    DROP COLUMN admin_upload_limit,
    DROP COLUMN admin_access,
    DROP COLUMN admin_public,
    DROP COLUMN admin_groups;

DROP TABLE admin_tokens;

-- ---------------------------------------------------------------------------
-- proxies, with their mocks folded in
-- ---------------------------------------------------------------------------

ALTER TABLE proxies ADD COLUMN document JSONB;

UPDATE proxies p
SET document = jsonb_strip_nulls(
        jsonb_build_object(
            'name', p.name,
            'type', p.kind,
            'url', p.url,
            'timeout', p.timeout_seconds,
            'body_limit', p.body_limit,
            'replace', p.replace_ratio,
            'rewrite_redirects', p.rewrite_redirects,
            -- `type`, not `kind`: the Rust field is `kind` and the wire name is
            -- `type`, and this document is read by the wire name.
            'resolve', jsonb_strip_nulls(
                jsonb_build_object('type', p.resolve_kind, 'header', p.resolve_header)
            ),
            'headers', p.headers,
            'access', p.access
        )
        || CASE
               WHEN p.loss_percentage IS NULL THEN '{}'::jsonb
               ELSE jsonb_build_object(
                        'loss',
                        jsonb_build_object(
                            'percentage', p.loss_percentage,
                            'status', p.loss_status
                        )
                    )
           END
        || CASE
               WHEN p.latency_percentage IS NULL THEN '{}'::jsonb
               ELSE jsonb_build_object(
                        'latency',
                        jsonb_build_object(
                            'percentage', p.latency_percentage,
                            'min', p.latency_min,
                            'max', p.latency_max
                        )
                    )
           END
        || jsonb_build_object(
               -- `[]` rather than absent. `save` omits an empty mock list and
               -- this writes it out, which is a difference in the JSON and not a
               -- difference in the configuration: both parse to the same empty
               -- vector. That is the invariant the whole backfill is held to --
               -- the document must *parse into* the configuration the old
               -- columns described, not match `save` byte for byte -- and it is
               -- what keeps every stored revision unchanged.
               'mocks', COALESCE(
                   (
                       SELECT jsonb_agg(
                                  jsonb_strip_nulls(
                                      jsonb_build_object(
                                          'name', m.name,
                                          'request', jsonb_build_object(
                                              'method', m.method,
                                              'url', m.url_pattern,
                                              'headers', m.request_headers,
                                              'query', m.request_query,
                                              'body', m.request_body
                                          ),
                                          'response', jsonb_strip_nulls(
                                              jsonb_build_object(
                                                  'status', m.status,
                                                  'body', m.body,
                                                  'json', m.json,
                                                  'template', m.template,
                                                  'headers', m.response_headers
                                              )
                                          ),
                                          'proxy', m.proxy_override
                                      )
                                  )
                                  ORDER BY m.ordinal
                              )
                       FROM mocks m
                       WHERE m.config = p.config AND m.proxy = p.name
                   ),
                   '[]'::jsonb
               )
           )
    );

ALTER TABLE proxies ALTER COLUMN document SET NOT NULL;

ALTER TABLE proxies
    DROP COLUMN kind,
    DROP COLUMN url,
    DROP COLUMN timeout_seconds,
    DROP COLUMN body_limit,
    DROP COLUMN replace_ratio,
    DROP COLUMN rewrite_redirects,
    DROP COLUMN resolve_kind,
    DROP COLUMN resolve_header,
    DROP COLUMN loss_percentage,
    DROP COLUMN loss_status,
    DROP COLUMN latency_percentage,
    DROP COLUMN latency_min,
    DROP COLUMN latency_max,
    DROP COLUMN headers,
    DROP COLUMN access;

DROP TABLE mocks;
