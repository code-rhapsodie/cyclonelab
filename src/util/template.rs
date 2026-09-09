/// Replaces `{@name}` placeholders with their value in a text.
///
/// Used to instantiate an SBOM template (`{@version}`, `{@date_now}`,
/// ...), but deliberately generic so it can serve other template-based
/// commands later.
pub fn render(template: &str, vars: &[(&str, &str)]) -> String {
    let mut result = template.to_string();
    for (key, value) in vars {
        result = result.replace(&format!("{{@{key}}}"), value);
    }
    result
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
        let out = render("hello {@name}!", &[("name", "world")]);
        assert_eq!(out, "hello world!");
    }

    #[test]
    fn leaves_unknown_placeholders_untouched() {
        let out = render("{@known} {@unknown}", &[("known", "ok")]);
        assert_eq!(out, "ok {@unknown}");
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
