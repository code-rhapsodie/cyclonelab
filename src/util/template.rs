/// Replaces `{key}` placeholders with their value in a text. The sigil
/// (`@`, `$`, ...) is part of `key` itself, so the same function serves
/// every placeholder namespace (`{@version}`, `{$repo}`, ...) simply by
/// prefixing the keys passed in `vars` before calling it.
pub fn render(template: &str, vars: &[(&str, &str)]) -> String {
    let mut result = template.to_string();
    for (key, value) in vars {
        result = result.replace(&format!("{{{key}}}"), value);
    }
    result
}

/// Like [`render`], but for a JSON template value (as used by `transform`'s
/// `item`/`build` step fields) instead of a plain string: substitution
/// walks every string leaf of `template`. A leaf that consists of exactly
/// one placeholder and nothing else is replaced by the referenced value
/// as-is, preserving its JSON type (e.g. an array stays an array); any
/// other string has its placeholders substituted textually, using the
/// referenced value's scalar representation.
pub fn render_value(
    template: &serde_json::Value,
    vars: &[(&str, &serde_json::Value)],
) -> serde_json::Value {
    use serde_json::Value;

    match template {
        Value::String(s) => {
            if let Some((_, v)) = vars.iter().find(|(name, _)| *s == format!("{{{name}}}")) {
                return (*v).clone();
            }
            let mut result = s.clone();
            for (name, v) in vars {
                let placeholder = format!("{{{name}}}");
                if result.contains(&placeholder) {
                    result = result.replace(&placeholder, &scalar_to_text(v));
                }
            }
            Value::String(result)
        }
        Value::Array(items) => Value::Array(items.iter().map(|v| render_value(v, vars)).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), render_value(v, vars)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Textual representation of a JSON scalar for use inside a bigger string
/// (e.g. `"urn:uuid:{@value}"`). Non-scalars fall back to their JSON
/// encoding, since embedding them verbatim in text is the caller's choice.
fn scalar_to_text(value: &serde_json::Value) -> String {
    use serde_json::Value;

    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Matches a single-wildcard `*` pattern (e.g. `"php_win32service*.zip"`)
/// against `filename`. Avoids a dependency on a glob crate for such a
/// simple need.
pub fn matches_single_wildcard(filename: &str, pattern: &str) -> bool {
    match pattern.split_once('*') {
        Some((prefix, suffix)) => {
            filename.len() >= prefix.len() + suffix.len()
                && filename.starts_with(prefix)
                && filename.ends_with(suffix)
        }
        None => filename == pattern,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_known_placeholders() {
        let out = render("hello {@name}!", &[("@name", "world")]);
        assert_eq!(out, "hello world!");
    }

    #[test]
    fn leaves_unknown_placeholders_untouched() {
        let out = render("{@known} {@unknown}", &[("@known", "ok")]);
        assert_eq!(out, "ok {@unknown}");
    }

    #[test]
    fn supports_different_sigil_namespaces() {
        let out = render("{$var} and {@value}", &[("$var", "a"), ("@value", "b")]);
        assert_eq!(out, "a and b");
    }

    #[test]
    fn render_value_substitutes_whole_value_preserving_type() {
        use serde_json::json;

        let template = json!({"ref": "{@item}", "name": "prefix-{@item.name}"});
        let item = json!({"name": "foo", "hashes": ["a", "b"]});
        let name = item["name"].clone();
        let vars: Vec<(&str, &serde_json::Value)> = vec![("@item", &item), ("@item.name", &name)];
        let out = render_value(&template, &vars);

        assert_eq!(out["ref"], item);
        assert_eq!(out["name"], "prefix-foo");
    }

    #[test]
    fn wildcard_matching() {
        assert!(matches_single_wildcard(
            "php_win32service-1.2.3.zip",
            "php_win32service*.zip"
        ));
        assert!(!matches_single_wildcard(
            "other.zip",
            "php_win32service*.zip"
        ));
        assert!(matches_single_wildcard("exact.zip", "exact.zip"));
    }
}
