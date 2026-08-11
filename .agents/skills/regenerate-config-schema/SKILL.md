---
name: regenerate-config-schema
description: Use after adding, removing or renaming any field in crates/doppel-core/src/config/, and before cutting a release. Regenerates doppel-config.schema.json and checks that every field still describes itself.
---

# Regenerate the configuration schema

`doppel-config.schema.json` at the repository root is generated, checked in,
attached to every release, and fetched by editors through a
`yaml-language-server` modeline. A stale copy is worse than none: it reports
mistakes that are not mistakes and accepts fields that no longer exist.

## Regenerate

```bash
uv run scripts/config_schema.py
uv run scripts/config_schema.py --check
```

The script runs `doppel config schema` and writes its output. Python tooling
here is driven by `uv`, never `pip`.

Two things already fail when the checked-in copy falls behind, so this skill is
about the cases they cannot see:

- `cargo test -p doppel-core --lib config::schema` compares the file to what the
  code produces, for whoever runs the suite locally;
- a CI step runs `--check`, so a forgotten regeneration cannot merge.

## Where the schema comes from

The same `utoipa::ToSchema` derives the admin API's OpenAPI document uses. There
is no second description of the types to keep in step, and that is the point --
so **do not** add `schemars`, a hand-written schema, or a second derive set.

A new type reachable from `Config` needs `utoipa::ToSchema` on it or the build
fails. Two standard-library types have no `utoipa` schema and are annotated
where they appear: `IpAddr` and `PathBuf` both carry
`#[schema(value_type = String)]`.

## Every field must describe itself

The schema is read in an editor. A field with no description is a tooltip that
says nothing, which is the whole reason it is generated from doc comments.

```bash
cargo test -p doppel-core --lib every_field_carries_a_description
```

That test enumerates rather than samples, so a field added without a `///` fails
it. Write the doc comment for the person editing YAML, not for the person
reading Rust: what the field is for, its unit, its default, and what happens
when it is left out.

`utoipa` puts a doc comment where you would not expect for an `Option<T>` whose
`T` has its own schema: the description lands *inside* the `oneOf` branch beside
the `$ref` rather than on the property. `config::schema::hoist_descriptions`
lifts it back out. If a field's description goes missing from the generated
file, that hoist is the first place to look -- not the doc comment.

## When the URL changes

`config::schema::URL` is the `$id` and is also the URL in
`main.example.yaml`'s modeline. They are compared by
`the_example_configs_modeline_names_this_url`, so changing one without the other
fails. It points at the raw file on `main` deliberately: a reader copies that
line once and keeps it, and a version-pinned URL would leave them validating
next year's configuration against an old schema. The per-release asset is there
for anyone who wants the pin.

## Check what it actually rejects

The generated file being current says nothing about it being useful. Validate a
document against it, and include a mistake:

```bash
uv run --with jsonschema --with pyyaml python - <<'EOF'
import json, yaml, jsonschema
schema = json.load(open("doppel-config.schema.json"))
jsonschema.Draft202012Validator.check_schema(schema)
doc = yaml.safe_load(open("main.example.yaml"))
print("example errors:", len(list(jsonschema.Draft202012Validator(schema).iter_errors(doc))))
doc["proxies"][0]["loss"]["percentage"] = 45   # a fraction was meant
print("with a bad percentage:", len(list(jsonschema.Draft202012Validator(schema).iter_errors(doc))))
EOF
```

The first count has to be zero and the second has to not be. A schema that
accepts everything passes every other check in this file.

## Report

Say whether the file changed, and name the fields whose descriptions you added
or reworded. "Regenerated, no diff" is a useful result; "the schema is fine" is
not.
