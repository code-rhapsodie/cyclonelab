//! Minimal JSONPath support for the `transform` command (see
//! `doc/transform/README.md` §5.1 for the exact subset covered):
//!
//! - `$` the document root.
//! - `.field` / `.field.sub` key access.
//! - `["field"]` / `['field']` key access, for names that aren't valid bare
//!   identifiers (e.g. `$.["$schema"]`).
//! - `[*]` every element of an array at this position.
//! - `..field` recursive descent: `field` captured at any depth.
//!
//! No numeric indices, slices, or `[?(...)]` filters: none of the recipes
//! in `schema/` need them, and adding full JSONPath support is deliberately
//! left for a day a real need shows up.
//!
//! A pattern is first parsed into [`Segment`]s, then resolved against a
//! document to a list of concrete, already-existing locations
//! ([`resolve`]). Reading, removing or overwriting a concrete location
//! re-walks the document from the root rather than holding long-lived
//! mutable references into it, which sidesteps borrow-checker conflicts
//! between locations that share a common array or object.

use std::fmt::Write as _;
use std::iter::Peekable;
use std::str::Chars;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// One step of an already-resolved, concrete location: a literal object key
/// or array index.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum PathElem {
    Key(String),
    Index(usize),
}

/// A concrete location in a JSON document, from the root down to one value.
pub type ConcretePath = Vec<PathElem>;

/// A parsed pattern segment, before resolution against any document.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Key(String),
    Wildcard,
    Recursive(String),
}

/// The JSON "type" a `when` guard can filter on (see action docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JsonType {
    String,
    Object,
    Array,
}

/// Whether `value`'s JSON type matches an optional `when` guard (no guard
/// always matches).
pub fn matches(value: &Value, when: Option<JsonType>) -> bool {
    match when {
        None => true,
        Some(JsonType::String) => value.is_string(),
        Some(JsonType::Object) => value.is_object(),
        Some(JsonType::Array) => value.is_array(),
    }
}

fn parse(path: &str) -> Result<Vec<Segment>> {
    let mut chars = path.chars().peekable();
    if chars.next() != Some('$') {
        bail!("JSONPath '{path}' must start with '$'");
    }

    let mut segments = Vec::new();
    while let Some(&c) = chars.peek() {
        match c {
            '.' => {
                chars.next();
                if chars.peek() == Some(&'[') {
                    // `.["field"]`: the dot is a no-op separator before a
                    // bracket segment (see `$.["$schema"]` in the docs).
                    continue;
                }
                let recursive = chars.peek() == Some(&'.');
                if recursive {
                    chars.next();
                }
                let ident = take_ident(&mut chars);
                if ident.is_empty() {
                    bail!(
                        "JSONPath '{path}': expected a field name after '{}'",
                        if recursive { ".." } else { "." }
                    );
                }
                segments.push(if recursive {
                    Segment::Recursive(ident)
                } else {
                    Segment::Key(ident)
                });
            }
            '[' => {
                chars.next();
                match chars.peek() {
                    Some('*') => {
                        chars.next();
                        expect_char(&mut chars, ']', path)?;
                        segments.push(Segment::Wildcard);
                    }
                    Some('"') | Some('\'') => {
                        let quote = chars.next().unwrap();
                        let mut key = String::new();
                        loop {
                            match chars.next() {
                                Some(c) if c == quote => break,
                                Some(c) => key.push(c),
                                None => bail!("JSONPath '{path}': unterminated string literal"),
                            }
                        }
                        expect_char(&mut chars, ']', path)?;
                        segments.push(Segment::Key(key));
                    }
                    _ => bail!("JSONPath '{path}': expected '*' or a quoted key after '['"),
                }
            }
            other => bail!("JSONPath '{path}': unexpected character '{other}'"),
        }
    }
    Ok(segments)
}

fn take_ident(chars: &mut Peekable<Chars>) -> String {
    let mut ident = String::new();
    while let Some(&c) = chars.peek() {
        if c.is_ascii_alphanumeric() || c == '_' {
            ident.push(c);
            chars.next();
        } else {
            break;
        }
    }
    ident
}

fn expect_char(chars: &mut Peekable<Chars>, expected: char, path: &str) -> Result<()> {
    match chars.next() {
        Some(c) if c == expected => Ok(()),
        _ => bail!("JSONPath '{path}': expected '{expected}'"),
    }
}

/// Resolves `path` against `doc`, returning the concrete location of every
/// value it currently matches. An empty result means the pattern matches
/// nothing yet (e.g. an absent field) — not an error, callers decide
/// whether that's a no-op or a problem.
pub fn resolve(doc: &Value, path: &str) -> Result<Vec<ConcretePath>> {
    let segments = parse(path)?;
    let mut out = Vec::new();
    resolve_rec(doc, ConcretePath::new(), &segments, &mut out);
    Ok(out)
}

fn resolve_rec(
    value: &Value,
    prefix: ConcretePath,
    segments: &[Segment],
    out: &mut Vec<ConcretePath>,
) {
    match segments.split_first() {
        None => out.push(prefix),
        Some((Segment::Key(name), rest)) => {
            if let Some(child) = value.as_object().and_then(|m| m.get(name)) {
                let mut p = prefix;
                p.push(PathElem::Key(name.clone()));
                resolve_rec(child, p, rest, out);
            }
        }
        Some((Segment::Wildcard, rest)) => {
            if let Some(arr) = value.as_array() {
                for (i, item) in arr.iter().enumerate() {
                    let mut p = prefix.clone();
                    p.push(PathElem::Index(i));
                    resolve_rec(item, p, rest, out);
                }
            }
        }
        Some((Segment::Recursive(name), rest)) => {
            let mut relative_matches = Vec::new();
            collect_recursive(value, name, &mut ConcretePath::new(), &mut relative_matches);
            for relative in relative_matches {
                if let Some(child) = get(value, &relative) {
                    let mut p = prefix.clone();
                    p.extend(relative);
                    resolve_rec(child, p, rest, out);
                }
            }
        }
    }
}

/// Collects, relative to `value`, every location (at any depth) where an
/// object has a key named `key`.
fn collect_recursive(
    value: &Value,
    key: &str,
    prefix: &mut ConcretePath,
    out: &mut Vec<ConcretePath>,
) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                prefix.push(PathElem::Key(k.clone()));
                if k == key {
                    out.push(prefix.clone());
                }
                collect_recursive(v, key, prefix, out);
                prefix.pop();
            }
        }
        Value::Array(arr) => {
            for (i, v) in arr.iter().enumerate() {
                prefix.push(PathElem::Index(i));
                collect_recursive(v, key, prefix, out);
                prefix.pop();
            }
        }
        _ => {}
    }
}

/// Reads the value at a concrete path, if present.
pub fn get<'a>(value: &'a Value, path: &[PathElem]) -> Option<&'a Value> {
    let mut current = value;
    for elem in path {
        current = match (elem, current) {
            (PathElem::Key(k), Value::Object(m)) => m.get(k)?,
            (PathElem::Index(i), Value::Array(a)) => a.get(*i)?,
            _ => return None,
        };
    }
    Some(current)
}

fn get_mut<'a>(value: &'a mut Value, path: &[PathElem]) -> Option<&'a mut Value> {
    let mut current = value;
    for elem in path {
        current = match (elem, current) {
            (PathElem::Key(k), Value::Object(m)) => m.get_mut(k)?,
            (PathElem::Index(i), Value::Array(a)) => a.get_mut(*i)?,
            _ => return None,
        };
    }
    Some(current)
}

/// Removes and returns the value at a concrete path. No-op (`None`) if any
/// segment along the way is already missing.
pub fn remove(doc: &mut Value, path: &[PathElem]) -> Option<Value> {
    let (last, parents) = path.split_last()?;
    let parent = get_mut(doc, parents)?;
    match (last, parent) {
        (PathElem::Key(k), Value::Object(m)) => m.remove(k),
        (PathElem::Index(i), Value::Array(a)) if *i < a.len() => Some(a.remove(*i)),
        _ => None,
    }
}

/// Removes every one of `paths` from `doc`. Locations sharing the same
/// parent array are removed in descending index order first, so removing
/// one doesn't shift the others out from under it.
pub fn remove_many(doc: &mut Value, mut paths: Vec<ConcretePath>) -> Vec<Value> {
    paths.sort();
    paths.reverse();
    paths.into_iter().filter_map(|p| remove(doc, &p)).collect()
}

/// Writes `value` at a concrete path, creating missing intermediate objects
/// for `Key` segments along the way (never arrays). Fails if a segment
/// would have to replace an existing non-object value, or index past the
/// end of an existing array by more than one.
pub fn set(doc: &mut Value, path: &[PathElem], value: Value) -> Result<()> {
    let Some((last, parents)) = path.split_last() else {
        *doc = value;
        return Ok(());
    };

    let mut current = doc;
    for elem in parents {
        match elem {
            PathElem::Key(k) => {
                if current.is_null() {
                    *current = Value::Object(Map::new());
                }
                let map = current.as_object_mut().with_context(|| {
                    format!("cannot descend into '{k}': an existing non-object value is in the way")
                })?;
                current = map
                    .entry(k.clone())
                    .or_insert_with(|| Value::Object(Map::new()));
            }
            PathElem::Index(i) => {
                let arr = current
                    .as_array_mut()
                    .context("cannot descend by index into a non-array value")?;
                current = arr
                    .get_mut(*i)
                    .with_context(|| format!("array index {i} does not exist"))?;
            }
        }
    }

    match last {
        PathElem::Key(k) => {
            if current.is_null() {
                *current = Value::Object(Map::new());
            }
            let map = current.as_object_mut().with_context(|| {
                format!("cannot set key '{k}': an existing non-object value is in the way")
            })?;
            map.insert(k.clone(), value);
        }
        PathElem::Index(i) => {
            let arr = current
                .as_array_mut()
                .context("cannot set an index into a non-array value")?;
            match (*i).cmp(&arr.len()) {
                std::cmp::Ordering::Less => arr[*i] = value,
                std::cmp::Ordering::Equal => arr.push(value),
                std::cmp::Ordering::Greater => bail!("array index {i} is out of bounds"),
            }
        }
    }
    Ok(())
}

/// Parses `path` and requires it to designate a single literal location (no
/// `*`/`..`) — used by `add`/`merge` targets, which must unambiguously
/// create missing structure.
pub fn literal(path: &str) -> Result<ConcretePath> {
    parse(path)?
        .into_iter()
        .map(|segment| match segment {
            Segment::Key(name) => Ok(PathElem::Key(name)),
            Segment::Wildcard => bail!("JSONPath '{path}': a wildcard ('[*]') is not allowed here"),
            Segment::Recursive(_) => {
                bail!("JSONPath '{path}': recursive descent ('..') is not allowed here")
            }
        })
        .collect()
}

/// Checks that `source` and `target` agree on every segment except a
/// possibly different trailing field name, as required for them to resolve
/// to the same set of parent nodes (see `doc/transform/README.md` §5.1).
/// Returns `target`'s trailing field name.
pub fn paired_target_key(source: &str, target: &str) -> Result<String> {
    let source_segments = parse(source)?;
    let target_segments = parse(target)?;

    if source_segments.is_empty() || target_segments.is_empty() {
        bail!("'{source}' and '{target}' must each designate at least one field");
    }
    if source_segments.len() != target_segments.len() {
        bail!(
            "'{source}' and '{target}' do not resolve to the same set of parent nodes (different depth)"
        );
    }
    let last = source_segments.len() - 1;
    if source_segments[..last] != target_segments[..last] {
        bail!("'{source}' and '{target}' do not resolve to the same set of parent nodes");
    }
    match (&source_segments[last], &target_segments[last]) {
        (Segment::Key(_), Segment::Key(name)) => Ok(name.clone()),
        (Segment::Recursive(_), Segment::Recursive(name)) => Ok(name.clone()),
        _ => bail!("'{source}' and '{target}' do not resolve to the same set of parent nodes"),
    }
}

/// A human-readable rendering of a concrete path, e.g. `$.metadata.component
/// .authors[0]`, for error and warning messages.
pub fn display(path: &[PathElem]) -> String {
    let mut out = String::from("$");
    for elem in path {
        match elem {
            PathElem::Key(k) if is_bare_ident(k) => {
                out.push('.');
                out.push_str(k);
            }
            PathElem::Key(k) => {
                let _ = write!(out, "[\"{k}\"]");
            }
            PathElem::Index(i) => {
                let _ = write!(out, "[{i}]");
            }
        }
    }
    out
}

fn is_bare_ident(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn dot_field_access() {
        let doc = json!({"a": 1});
        let result = resolve(&doc, "$.a").unwrap();
        assert_eq!(result, vec![vec![PathElem::Key("a".into())]]);
        assert_eq!(get(&doc, &result[0]), Some(&json!(1)));
    }

    #[test]
    fn nested_dot_field_access() {
        let doc = json!({"a": {"b": 2}});
        let result = resolve(&doc, "$.a.b").unwrap();
        assert_eq!(get(&doc, &result[0]), Some(&json!(2)));
    }

    #[test]
    fn bracket_key_access_for_non_identifier_names() {
        let doc = json!({"$schema": "value"});
        let result = resolve(&doc, "$.[\"$schema\"]").unwrap();
        assert_eq!(get(&doc, &result[0]), Some(&json!("value")));
    }

    #[test]
    fn wildcard_over_array() {
        let doc = json!({"a": [{"b": 1}, {"b": 2}, {}]});
        let result = resolve(&doc, "$.a[*].b").unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(get(&doc, &result[0]), Some(&json!(1)));
        assert_eq!(get(&doc, &result[1]), Some(&json!(2)));
    }

    #[test]
    fn recursive_descent_at_any_depth() {
        let doc = json!({
            "author": "top",
            "components": [
                {"author": "nested"},
                {"commit": {"author": {"name": "not-a-match-target"}}}
            ]
        });
        let result = resolve(&doc, "$..author").unwrap();
        let values: Vec<_> = result
            .iter()
            .map(|p| get(&doc, p).unwrap().clone())
            .collect();
        assert_eq!(values.len(), 3);
        assert!(values.contains(&json!("top")));
        assert!(values.contains(&json!("nested")));
        assert!(values.contains(&json!({"name": "not-a-match-target"})));
    }

    #[test]
    fn no_match_returns_empty_result() {
        let doc = json!({"a": 1});
        assert!(resolve(&doc, "$.missing").unwrap().is_empty());
        assert!(resolve(&doc, "$..missing").unwrap().is_empty());
        assert!(resolve(&doc, "$.a[*]").unwrap().is_empty());
    }

    #[test]
    fn set_creates_missing_intermediate_objects() {
        let mut doc = json!({});
        set(&mut doc, &literal("$.a.b").unwrap(), json!(42)).unwrap();
        assert_eq!(doc, json!({"a": {"b": 42}}));
    }

    #[test]
    fn set_overwrites_an_existing_value() {
        let mut doc = json!({"a": {"b": 1}});
        set(&mut doc, &literal("$.a.b").unwrap(), json!(2)).unwrap();
        assert_eq!(doc, json!({"a": {"b": 2}}));
    }

    #[test]
    fn remove_many_handles_shared_parent_array_without_shifting_bugs() {
        let mut doc = json!({"items": [1, 2, 3]});
        let paths = vec![
            vec![PathElem::Key("items".into()), PathElem::Index(0)],
            vec![PathElem::Key("items".into()), PathElem::Index(2)],
        ];
        remove_many(&mut doc, paths);
        assert_eq!(doc, json!({"items": [2]}));
    }

    #[test]
    fn remove_is_a_noop_when_absent() {
        let mut doc = json!({"a": 1});
        assert_eq!(remove(&mut doc, &[PathElem::Key("missing".into())]), None);
        assert_eq!(doc, json!({"a": 1}));
    }

    #[test]
    fn literal_rejects_wildcards_and_recursive_descent() {
        assert!(literal("$.a[*]").is_err());
        assert!(literal("$..a").is_err());
        assert!(literal("$.a.b").is_ok());
    }

    #[test]
    fn paired_target_key_requires_identical_parent_segments() {
        assert_eq!(
            paired_target_key("$.metadata.manufacture", "$.metadata.manufacturer").unwrap(),
            "manufacturer"
        );
        assert!(paired_target_key("$..author", "$.other").is_err());
        assert!(paired_target_key("$.a.b", "$.a[*]").is_err());
    }

    #[test]
    fn display_renders_bracket_syntax_for_non_identifier_keys() {
        let path = vec![PathElem::Key("$schema".into())];
        assert_eq!(display(&path), "$[\"$schema\"]");
        let path = vec![
            PathElem::Key("a".into()),
            PathElem::Index(2),
            PathElem::Key("b".into()),
        ];
        assert_eq!(display(&path), "$.a[2].b");
    }
}
