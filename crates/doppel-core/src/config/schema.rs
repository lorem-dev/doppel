//! The configuration document described as a JSON Schema, for editors and for
//! anything that validates a file before Doppel sees it.
//!
//! Derived from the same `utoipa::ToSchema` implementations the admin API's
//! OpenAPI document uses, rather than from a second set of derives or a
//! hand-written file. Two descriptions of one type drift, and the one nobody
//! runs drifts first; this way a field added to `Config` appears here or the
//! drift test in `tests` below fails.
//!
//! `utoipa` 5 emits OpenAPI 3.1 schema objects, and 3.1 aligned its schema
//! dialect with JSON Schema 2020-12 -- so the objects need no translation, only
//! rehousing: OpenAPI keeps its definitions under `#/components/schemas/` and
//! JSON Schema under `#/$defs/`, so every `$ref` is rewritten.

use serde_json::{Value, json};

/// Where the checked-in copy of this schema is fetched from.
///
/// The raw file on `main`, not a release asset: this URL goes into a `$schema`
/// comment that a reader copies once and keeps, and pinning it to whichever
/// version happened to be current would leave them validating next year's file
/// against an old schema. Every release also attaches the file, for anyone who
/// wants the pin.
pub const URL: &str =
    "https://raw.githubusercontent.com/lorem-dev/doppel/main/doppel-config.schema.json";

/// The whole configuration document as a JSON Schema 2020-12 object.
#[must_use]
pub fn json_schema() -> Value {
    let mut defs = Vec::new();
    <super::Config as utoipa::ToSchema>::schemas(&mut defs);

    let mut root = serde_json::to_value(<super::Config as utoipa::PartialSchema>::schema())
        .expect("a utoipa schema serializes");
    rehouse_refs(&mut root);
    hoist_descriptions(&mut root);

    let mut definitions = serde_json::Map::new();
    for (name, schema) in defs {
        let mut value = serde_json::to_value(schema).expect("a utoipa schema serializes");
        rehouse_refs(&mut value);
        hoist_descriptions(&mut value);
        definitions.insert(name, value);
    }

    let mut out = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": URL,
        "title": "Doppel configuration",
        "description": concat!(
            "The `main.yaml` Doppel reads. Generated from the Rust types by ",
            "`doppel config schema`; do not edit by hand."
        ),
    });
    let object = out.as_object_mut().expect("built from a json! object");
    // Insertion order does not survive: `serde_json`'s map is a `BTreeMap`
    // unless `preserve_order` is on, so the emitted file is sorted by key
    // whatever order things are added in. That is worth having for a generated
    // file -- the diff between two versions is the change and nothing else --
    // and it is why the metadata keys appear among the schema's own rather than
    // above them.
    if let Value::Object(root) = root {
        object.extend(root);
    }
    object.insert("$defs".to_owned(), Value::Object(definitions));
    out
}

/// The schema as the file on disk holds it: pretty-printed, one trailing
/// newline. Shared by the CLI and the drift test so neither can disagree about
/// formatting.
#[must_use]
pub fn json_schema_document() -> String {
    let mut text = serde_json::to_string_pretty(&json_schema()).expect("a json value serializes");
    text.push('\n');
    text
}

/// Lifts a field's description out of the `oneOf` branch `utoipa` buries it in,
/// up to the field itself.
///
/// An `Option<T>` where `T` is its own schema is emitted as
/// `{"oneOf": [{"type": "null"}, {"$ref": "...", "description": "..."}]}` -- the
/// doc comment is there, one level below where anything looks for it. An editor
/// showing a tooltip for `timeout:` reads the property, finds no description and
/// shows nothing, which is the whole reason those doc comments were written.
///
/// Only the description moves; the `oneOf` stays, so the field is still
/// nullable. JSON Schema 2020-12 allows keywords beside a `$ref`, which
/// draft-07 did not -- worth knowing, because it is why this can be a hoist
/// rather than a restructuring.
fn hoist_descriptions(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if !map.contains_key("description")
                && let Some(Value::Array(branches)) = map.get_mut("oneOf")
            {
                let lifted = branches.iter_mut().find_map(|branch| {
                    branch
                        .as_object_mut()
                        .filter(|b| b.contains_key("$ref"))
                        .and_then(|b| b.remove("description"))
                });
                if let Some(description) = lifted {
                    map.insert("description".to_owned(), description);
                }
            }
            for nested in map.values_mut() {
                hoist_descriptions(nested);
            }
        }
        Value::Array(items) => {
            for item in items {
                hoist_descriptions(item);
            }
        }
        _ => {}
    }
}

/// Rewrites every `$ref` from OpenAPI's location to JSON Schema's, in place and
/// at any depth.
fn rehouse_refs(value: &mut Value) {
    const OPENAPI: &str = "#/components/schemas/";
    match value {
        Value::Object(map) => {
            if let Some(Value::String(reference)) = map.get_mut("$ref")
                && let Some(name) = reference.strip_prefix(OPENAPI)
            {
                *reference = format!("#/$defs/{name}");
            }
            for nested in map.values_mut() {
                rehouse_refs(nested);
            }
        }
        Value::Array(items) => {
            for item in items {
                rehouse_refs(item);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The checked-in file is what editors and the release fetch, so it has to
    /// be what the code produces. Regenerate with:
    ///
    /// ```text
    /// uv run scripts/config_schema.py
    /// ```
    #[test]
    fn the_checked_in_schema_is_what_the_code_generates() {
        let on_disk = include_str!("../../../../doppel-config.schema.json");
        assert_eq!(
            on_disk,
            json_schema_document(),
            "doppel-config.schema.json is stale; regenerate it with \
             `uv run scripts/config_schema.py`"
        );
    }

    #[test]
    fn no_ref_still_points_at_the_openapi_component_path() {
        let text = json_schema_document();
        assert!(
            !text.contains("#/components/schemas/"),
            "a $ref was left pointing into an OpenAPI document"
        );
        assert!(text.contains("#/$defs/"), "no $ref was rehoused at all");
    }

    /// Every `$ref` has to resolve, or an editor reports the file as broken
    /// rather than reporting the mistake in the configuration.
    #[test]
    fn every_ref_resolves_to_a_definition() {
        let schema = json_schema();
        let defs = schema["$defs"].as_object().expect("$defs is an object");

        let mut refs = Vec::new();
        collect_refs(&schema, &mut refs);
        assert!(!refs.is_empty(), "the schema has no $refs to check");

        let dangling: Vec<_> = refs
            .iter()
            .filter_map(|r| r.strip_prefix("#/$defs/"))
            .filter(|name| !defs.contains_key(*name))
            .collect();
        assert!(dangling.is_empty(), "unresolved $refs: {dangling:?}");
    }

    /// The sections a reader edits first. Named individually rather than
    /// counted, so adding a section cannot quietly satisfy the assertion.
    #[test]
    fn the_top_level_sections_are_all_described() {
        let schema = json_schema();
        let properties = schema["properties"]
            .as_object()
            .expect("the root describes properties");
        for section in [
            "server",
            "logging",
            "control",
            "templates",
            "sentry",
            "admin",
            "proxies",
        ] {
            assert!(
                properties.contains_key(section),
                "`{section}` is missing from the schema root: {:?}",
                properties.keys().collect::<Vec<_>>()
            );
        }
    }

    /// The schema exists to be read in an editor, and a field with no
    /// description is a tooltip that says nothing. Enumerated rather than
    /// spot-checked: a field added without a doc comment fails here, which is
    /// the only moment anyone would notice.
    #[test]
    fn every_field_carries_a_description() {
        let schema = json_schema();
        let mut bare = Vec::new();

        let mut check = |owner: &str, node: &Value| {
            if let Some(properties) = node.get("properties").and_then(Value::as_object) {
                for (field, spec) in properties {
                    if spec.get("description").is_none() {
                        bare.push(format!("{owner}.{field}"));
                    }
                }
            }
        };
        check("Config", &schema);
        for (name, node) in schema["$defs"].as_object().expect("$defs is an object") {
            check(name, node);
        }

        assert!(
            bare.is_empty(),
            "these fields would show an empty tooltip; give them a doc comment: {bare:?}"
        );
    }

    /// `main.example.yaml` carries the URL in a `yaml-language-server` modeline,
    /// and a reader copies that line into their own file. If the two disagree,
    /// every copy points somewhere wrong.
    #[test]
    fn the_example_configs_modeline_names_this_url() {
        let example = include_str!("../../../../main.example.yaml");
        let modeline = example
            .lines()
            .find(|line| line.contains("yaml-language-server"))
            .expect("main.example.yaml must carry a $schema modeline");
        assert!(
            modeline.contains(URL),
            "the modeline names a different URL than `schema::URL`:\n  {modeline}"
        );
    }

    fn collect_refs(value: &Value, out: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                if let Some(Value::String(reference)) = map.get("$ref") {
                    out.push(reference.clone());
                }
                for nested in map.values() {
                    collect_refs(nested, out);
                }
            }
            Value::Array(items) => {
                for item in items {
                    collect_refs(item, out);
                }
            }
            _ => {}
        }
    }
}
