//! An asset's other files (its `representations` list), read and carried
//! forward as Arbesk's asset-core does (`manifest/representations.ts`) and
//! as `besk attach` writes them. Everything works on the stored
//! `serde_json::Value`, so a manifest is never re-serialised through a
//! struct that would drop a key this app doesn't know.

use serde_json::{Value, json};

// ---- Versions -----------------------------------------------------------------

/// besk's `versionOf`: the manifest's top-level `version` when it is a JSON
/// number (any form: `3`, `3.0`, `-1`), else 1 (a string, missing).
pub(crate) fn version_of(manifest: &Value) -> f64 {
    manifest["version"].as_f64().unwrap_or(1.0)
}

/// The version after `manifest`'s (`version_of + 1`), written as an
/// integer when it is one (`4`, not `4.0`), as JavaScript writes it.
pub(crate) fn next_version(manifest: &Value) -> Value {
    let next = version_of(manifest) + 1.0;
    // Whole numbers JavaScript would print without a fraction, as i64.
    let whole = next.fract() == 0.0 && next.abs() < 9_007_199_254_740_992.0;
    if whole { json!(next as i64) } else { json!(next) }
}

// ---- Carrying the list forward ---------------------------------------------------

/// What a new version of `manifest`'s model carries forward: its
/// `representations` value verbatim when it is a non-empty JSON array, as
/// besk's `nextModelEntry` does; otherwise nothing (no key, `[]`, `null`,
/// an object or any other non-array), so the key is left out rather than
/// written as something `manifestSchema` would refuse.
pub(crate) fn carried(manifest: &Value) -> Option<Value> {
    match &manifest["representations"] {
        Value::Array(list) if !list.is_empty() => Some(Value::Array(list.clone())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_of_reads_any_json_number_else_one() {
        for (version, of, next) in [
            (json!(3), 3.0, json!(4)),
            (json!(3.0), 3.0, json!(4)),
            (json!(-1), -1.0, json!(0)),
            (json!(2.5), 2.5, json!(3.5)),
            (json!("3"), 1.0, json!(2)),
            (Value::Null, 1.0, json!(2)),
        ] {
            let manifest = json!({ "version": version });
            assert_eq!(version_of(&manifest), of, "{version}");
            assert_eq!(next_version(&manifest), next, "{version}");
        }
        assert_eq!(version_of(&json!({})), 1.0);
        assert_eq!(next_version(&json!({})), json!(2));
        // `4`, never `4.0`.
        assert_eq!(next_version(&json!({ "version": 3.0 })).to_string(), "4");
    }

    #[test]
    fn only_a_non_empty_array_is_carried() {
        let list = json!([{ "id": "stl", "future": { "kept": true } }, "not even an entry"]);
        assert_eq!(carried(&json!({ "representations": list })), Some(list));
        for value in [json!([]), Value::Null, json!({}), json!("x"), json!(3)] {
            assert_eq!(carried(&json!({ "representations": value })), None, "{value}");
        }
        assert_eq!(carried(&json!({})), None);
    }
}
