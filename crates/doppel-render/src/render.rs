//! Rendering: a template string and a set of bound [`Variables`] in,
//! rendered bytes or a `TEMPLATE_RENDER_ERROR` out.
//!
//! `render_json` is deliberately not `render_str` under another name: it
//! parses the rendered output as JSON and re-serializes it, so a `json:`
//! mock field that renders to malformed JSON fails loudly instead of
//! shipping a broken body. See section 7 of the design.

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

    /// Renders `template`, then parses the output as JSON and re-serializes
    /// it.
    ///
    /// A template that renders successfully but whose output is not valid
    /// JSON is `TEMPLATE_RENDER_ERROR`, same as a template syntax error, but
    /// the message says so explicitly: the mistake is in what the template
    /// *produced*, not in the template itself, and an operator fixes those
    /// two differently.
    pub fn render_json(&self, template: &str, vars: &Variables) -> Result<String, Error> {
        let rendered = self.render_str(template, vars)?;

        let value: serde_json::Value = serde_json::from_str(&rendered).map_err(|err| {
            Error::new(
                ErrorCode::TemplateRenderError,
                format!("template rendered successfully but its output is not valid json: {err}"),
            )
        })?;

        serde_json::to_string(&value).map_err(|err| {
            Error::new(
                ErrorCode::TemplateRenderError,
                format!("rendered json value could not be re-serialized: {err}"),
            )
        })
    }
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}

/// Turns a minijinja error into `TEMPLATE_RENDER_ERROR`.
///
/// For an undefined variable, minijinja's own message is just "undefined
/// value" with no indication of which variable -- the name is lost by the
/// time the VM notices the value is undefined. minijinja does keep the byte
/// range of the failing expression and the template source when debug mode
/// is on (forced on in `new`, above), so that range is sliced out of the
/// source to name the variable explicitly. Every other error kind --
/// syntax errors, a filter applied to the wrong type, bad arithmetic --
/// already carries a specific, readable message from minijinja itself, so
/// those are passed through as-is.
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

    Error::new(ErrorCode::TemplateRenderError, err.to_string())
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
}
