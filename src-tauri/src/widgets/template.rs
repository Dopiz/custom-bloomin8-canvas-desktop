//! `{{PLACEHOLDER}}` substitution shared by every widget renderer.
//!
//! Ports the simple `str.replace(...)` chains used by the Python reference
//! scripts (`render.py`/`weather.py`/`countdown.py`), plus one guard rail
//! they don't have: if any `{{...}}` survives substitution (typo'd key,
//! template/renderer drift), that's a bug, and we fail loudly instead of
//! shipping half-templated HTML to the device.

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TemplateError {
    #[error("template still has unresolved placeholders after substitution: {0}")]
    UnresolvedPlaceholder(String),
}

/// Replace every `{{KEY}}` in `template` with its matching value from
/// `values`, then verify no `{{PLACEHOLDER_TOKEN}}`-shaped text survives.
///
/// The leftover-check only flags tokens that look like our placeholder
/// syntax (`{{`, then one or more `A-Z`/`_`, then `}}`) rather than any bare
/// `"{{"` substring. This matters because at least one bundled template
/// (`crypto-template.html`) has a CSS *comment* mentioning the literal text
/// `{{COLS_*}}` to document where render.py injects values — that's not an
/// unresolved substitution, just prose, and a naive `contains("{{")` check
/// would false-positive on it forever.
pub fn render(template: &str, values: &[(&str, &str)]) -> Result<String, TemplateError> {
    let mut out = template.to_string();
    for (key, value) in values {
        out = out.replace(&format!("{{{{{key}}}}}"), value);
    }
    if let Some(token) = find_placeholder_token(&out) {
        return Err(TemplateError::UnresolvedPlaceholder(token.to_string()));
    }
    Ok(out)
}

/// Finds the first `{{TOKEN}}`-shaped substring where `TOKEN` is one or more
/// ASCII uppercase letters/underscores (our placeholder key alphabet).
/// Text that merely contains `"{{"` without that shape (e.g. prose in a CSS
/// comment) is not considered a leftover placeholder.
fn find_placeholder_token(s: &str) -> Option<&str> {
    let mut search_from = 0;
    while let Some(rel_start) = s[search_from..].find("{{") {
        let start = search_from + rel_start;
        let after_open = start + 2;
        if let Some(rel_end) = s[after_open..].find("}}") {
            let inner = &s[after_open..after_open + rel_end];
            let end = after_open + rel_end + 2;
            if !inner.is_empty()
                && inner
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c == '_')
            {
                return Some(&s[start..end]);
            }
            search_from = after_open + rel_end + 2;
        } else {
            // No closing "}}" anywhere after this "{{"; nothing further to find.
            break;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitutes_all_placeholders() {
        let tpl = "hello {{NAME}}, you are {{AGE}}";
        let out = render(tpl, &[("NAME", "Ada"), ("AGE", "36")]).unwrap();
        assert_eq!(out, "hello Ada, you are 36");
    }

    #[test]
    fn errors_on_leftover_placeholder() {
        let tpl = "hello {{NAME}}, you are {{AGE}}";
        let err = render(tpl, &[("NAME", "Ada")]).unwrap_err();
        assert_eq!(err, TemplateError::UnresolvedPlaceholder("{{AGE}}".to_string()));
    }

    #[test]
    fn prose_mentioning_literal_braces_does_not_trip_the_leftover_guard() {
        // A value (or template comment) containing "{{ ... }}" that isn't
        // shaped like our placeholder syntax (ALL_CAPS/underscore key) must
        // not be mistaken for an unresolved substitution — this is exactly
        // the crypto template's CSS comment mentioning "{{COLS_*}}".
        let tpl = "note: {{NOTE}}";
        let out = render(tpl, &[("NOTE", "use {{ and }} in prose")]).unwrap();
        assert_eq!(out, "note: use {{ and }} in prose");
    }

    #[test]
    fn a_genuinely_unresolved_placeholder_shaped_token_still_errors() {
        let tpl = "note: {{NOTE}} also {{SOME_KEY}}";
        let err = render(tpl, &[("NOTE", "ok")]).unwrap_err();
        assert_eq!(
            err,
            TemplateError::UnresolvedPlaceholder("{{SOME_KEY}}".to_string())
        );
    }
}
