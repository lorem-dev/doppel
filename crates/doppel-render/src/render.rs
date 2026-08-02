//! Rendering: a template string and a set of bound [`Variables`] in,
//! rendered bytes or a `TEMPLATE_RENDER_ERROR` out.
//!
//! `render_json` is deliberately not `render_str` under another name: it
//! also parses the rendered output as JSON, purely to validate it, so a
//! `json:` mock field that renders to malformed JSON fails loudly instead of
//! shipping a broken body. The rendered string itself is returned unchanged
//! -- validation must not rewrite the bytes a mock author wrote. See section
//! 7 of the design.

use doppel_core::{Error, ErrorCode};

use crate::extract::Variables;

/// The shared rendering environment, configured once at construction.
pub struct Renderer(minijinja::Environment<'static>);

impl Renderer {
    /// Builds the rendering environment.
    ///
    /// `UndefinedBehavior::Strict` is set here, once: an undefined variable
    /// in a template is an error, not an empty string. minijinja's default
    /// is permissive, so this has to be explicit -- the same argument as
    /// `deny_unknown_fields` on the config model. A mock that silently
    /// renders `"name": ""` because someone mistyped a variable is worse
    /// than one that fails.
    ///
    /// Debug mode is also forced on here, regardless of build profile.
    /// minijinja defaults it to `cfg!(debug_assertions)`, which is off in a
    /// release build; without forcing it, the source span `render_error`
    /// below relies on to name an undefined variable would be unavailable
    /// in exactly the build an operator runs in production.
    #[must_use]
    pub fn new() -> Self {
        let mut env = minijinja::Environment::new();
        env.set_undefined_behavior(minijinja::UndefinedBehavior::Strict);
        env.set_debug(true);
        Self(env)
    }

    /// Renders `template` against `vars`, returning the raw output.
    ///
    /// Never panics: a syntax error, an undefined variable, a filter
    /// applied to the wrong type or any other minijinja failure becomes
    /// `TEMPLATE_RENDER_ERROR` rather than unwinding. Rendering runs on the
    /// request path with client-influenced variable values, so a panic here
    /// would take down a worker.
    pub fn render_str(&self, template: &str, vars: &Variables) -> Result<String, Error> {
        self.0
            .render_str(template, vars.as_context())
            .map_err(render_error)
    }

    /// Renders `template`, then parses the output as JSON purely to
    /// validate it, and returns the rendered string unchanged.
    ///
    /// This deliberately does not re-serialize: a mock exists to reproduce a
    /// backend's bytes as closely as it can, so rewriting the key order or
    /// whitespace of JSON the template author wrote would make this a worse
    /// stand-in for no gain, and would fight byte-for-byte assertions against
    /// it. Parsing exists only to catch the case this method is for: a
    /// template that renders successfully but whose output is not valid
    /// JSON. That failure is `TEMPLATE_RENDER_ERROR`, same code as a template
    /// syntax error, but the message says so explicitly: the mistake is in
    /// what the template *produced*, not in the template itself, and an
    /// operator fixes those two differently.
    pub fn render_json(&self, template: &str, vars: &Variables) -> Result<String, Error> {
        let rendered = self.render_str(template, vars)?;

        serde_json::from_str::<serde_json::Value>(&rendered).map_err(|err| {
            Error::new(
                ErrorCode::TemplateRenderError,
                format!("template rendered successfully but its output is not valid json: {err}"),
            )
        })?;

        Ok(rendered)
    }
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}

/// Turns a minijinja error into `TEMPLATE_RENDER_ERROR`.
///
/// For a bare undefined variable (`ErrorKind::UndefinedError`), minijinja's
/// own message is just "undefined value" with no indication of which
/// variable -- the name is lost by the time the VM notices the value is
/// undefined. minijinja does keep the byte range of the failing expression
/// and the template source when debug mode is on (forced on in `new`,
/// above), so that range is sliced out of the source to name the variable
/// explicitly. That is the best message available for this kind, so it is
/// used whenever it is available.
///
/// An undefined value used as the operand of a filter or a binary operator
/// does not take this path, though -- minijinja raises `InvalidOperation`
/// there instead, with a message that says a value was undefined but never
/// which one (confirmed empirically against minijinja 2.21.0: `{{ missing |
/// length }}` and `{{ missing + 1 }}` both land here). Chasing the specific
/// set of error kinds that can carry an undefined operand would mean losing
/// every time minijinja adds one, so instead every render error, of any
/// kind, gets the template line it failed on attached to its message via
/// `render_snippet`. The byte range itself is too narrow for this: for a
/// filter it covers only the filter's name, not its argument, so slicing
/// exactly that range would still drop the variable. The whole source line
/// keeps the variable visible regardless of which sub-expression minijinja
/// chose to blame.
fn render_error(err: minijinja::Error) -> Error {
    if err.kind() == minijinja::ErrorKind::UndefinedError
        && let (Some(range), Some(source)) = (err.range(), err.template_source())
        && let Some(expr) = source.get(range)
    {
        return Error::new(
            ErrorCode::TemplateRenderError,
            format!("template references undefined variable `{expr}`"),
        );
    }

    match render_snippet(&err) {
        Some(snippet) => Error::new(
            ErrorCode::TemplateRenderError,
            format!("{err} (in expression: {snippet})"),
        ),
        None => Error::new(ErrorCode::TemplateRenderError, err.to_string()),
    }
}

/// The template source line a render error occurred on, trimmed of
/// surrounding whitespace. `None` when minijinja did not attach line
/// information, or debug mode did not capture the source to slice it from
/// (should not happen here, since `Renderer::new` forces debug mode on).
fn render_snippet(err: &minijinja::Error) -> Option<&str> {
    let line = err.line()?;
    let source = err.template_source()?;
    source.lines().nth(line - 1).map(str::trim)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars_from(pairs: &[(&str, serde_json::Value)]) -> Variables {
        let mut vars = Variables::new();
        for (name, value) in pairs {
            vars.insert(name, value.clone());
        }
        vars
    }

    #[test]
    fn a_plain_substitution_renders() {
        let renderer = Renderer::new();
        let vars = vars_from(&[("name", serde_json::json!("ada"))]);

        let out = renderer.render_str("hello {{ name }}", &vars).unwrap();

        assert_eq!(out, "hello ada");
    }

    #[test]
    fn a_filter_over_an_array_bound_from_a_selector_renders() {
        let renderer = Renderer::new();
        let vars = vars_from(&[("items", serde_json::json!([1, 2, 3]))]);

        let out = renderer.render_str("{{ items | length }}", &vars).unwrap();

        assert_eq!(out, "3");
    }

    #[test]
    fn an_undefined_variable_errors_and_the_message_names_it() {
        let renderer = Renderer::new();
        let vars = Variables::new();

        let err = renderer
            .render_str("hello {{ missing_name }}", &vars)
            .unwrap_err();

        assert_eq!(err.code, ErrorCode::TemplateRenderError);
        assert!(
            err.message.contains("missing_name"),
            "message did not name the undefined variable: {}",
            err.message
        );
    }

    #[test]
    fn render_json_accepts_output_that_is_valid_json() {
        let renderer = Renderer::new();
        let vars = vars_from(&[("id", serde_json::json!(42))]);

        let out = renderer.render_json(r#"{"id": {{ id }}}"#, &vars).unwrap();

        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&out).unwrap(),
            serde_json::json!({"id": 42})
        );
    }

    #[test]
    fn render_json_preserves_key_order_and_spacing_rather_than_re_serializing() {
        let renderer = Renderer::new();
        let vars = vars_from(&[
            ("id", serde_json::json!(42)),
            ("name", serde_json::json!("ada")),
        ]);

        let template = r#"{ "zebra": true, "name": "{{ name }}", "id": {{ id }} }"#;
        let out = renderer.render_json(template, &vars).unwrap();

        assert_eq!(out, r#"{ "zebra": true, "name": "ada", "id": 42 }"#);
    }

    #[test]
    fn render_json_rejects_output_that_is_not_json_with_a_distinguishable_message() {
        let renderer = Renderer::new();
        let vars = vars_from(&[("name", serde_json::json!("ada"))]);

        let err = renderer.render_json("hello {{ name }}", &vars).unwrap_err();

        assert_eq!(err.code, ErrorCode::TemplateRenderError);
        assert!(
            err.message.contains("its output is not valid json"),
            "message did not distinguish a bad-output failure from a syntax error: {}",
            err.message
        );

        let syntax_err = renderer.render_json("{{ broken", &vars).unwrap_err();
        assert_ne!(err.message, syntax_err.message);
    }

    #[test]
    fn a_template_with_broken_syntax_errors_rather_than_panicking() {
        let renderer = Renderer::new();
        let vars = Variables::new();

        let err = renderer.render_str("{{ if broken", &vars).unwrap_err();

        assert_eq!(err.code, ErrorCode::TemplateRenderError);
    }

    #[test]
    fn a_filter_misapplied_to_the_wrong_type_errors_rather_than_panicking() {
        let renderer = Renderer::new();
        let vars = vars_from(&[("count", serde_json::json!(5))]);

        let err = renderer
            .render_str("{{ count | length }}", &vars)
            .unwrap_err();

        assert_eq!(err.code, ErrorCode::TemplateRenderError);
    }

    #[test]
    fn an_undefined_variable_behind_a_filter_errors_and_the_message_identifies_it() {
        // The exact expression main.example.yaml uses: `length` over a
        // body-extracted array. minijinja raises `InvalidOperation` here,
        // not `UndefinedError`, so the message must not depend on the
        // special-cased undefined path to name the variable.
        let renderer = Renderer::new();
        let vars = Variables::new();

        let err = renderer
            .render_str("{{ resourceItems | length }}", &vars)
            .unwrap_err();

        assert_eq!(err.code, ErrorCode::TemplateRenderError);
        assert!(
            err.message.contains("resourceItems"),
            "message did not identify the undefined variable behind the filter: {}",
            err.message
        );
    }

    #[test]
    fn an_undefined_variable_in_a_binary_operation_errors_and_the_message_identifies_it() {
        // Also `InvalidOperation`, not `UndefinedError`: an operator applied
        // to an undefined operand loses the name the same way a filter does.
        let renderer = Renderer::new();
        let vars = Variables::new();

        let err = renderer.render_str("{{ missing + 1 }}", &vars).unwrap_err();

        assert_eq!(err.code, ErrorCode::TemplateRenderError);
        assert!(
            err.message.contains("missing"),
            "message did not identify the undefined variable in the operation: {}",
            err.message
        );
    }

    #[test]
    fn a_render_error_with_no_undefined_operand_still_carries_the_expression_snippet() {
        // Neither operand here is undefined -- `count` is bound, just to the
        // wrong type. This pins the general snippet mechanism itself rather
        // than a side effect of naming an undefined variable: the message
        // must carry the failing expression for any render error, not only
        // ones that happen to involve an undefined value.
        let renderer = Renderer::new();
        let vars = vars_from(&[("count", serde_json::json!(5))]);

        let err = renderer
            .render_str("{{ count | length }}", &vars)
            .unwrap_err();

        assert_eq!(err.code, ErrorCode::TemplateRenderError);
        assert!(
            err.message.contains("count") && err.message.contains("length"),
            "message did not carry the offending template snippet: {}",
            err.message
        );
    }
}
