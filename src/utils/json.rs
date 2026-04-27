//! JSON utility functions

use anyhow::Result;
use serde_json::Value;

/// Safely get a nested JSON value using a dot-separated path
pub fn get_nested<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let parts: Vec<&str> = path.split('.').collect();
    let mut current = value;

    for part in parts {
        current = match current {
            Value::Object(map) => map.get(part),
            Value::Array(arr) => {
                if let Ok(index) = part.parse::<usize>() {
                    arr.get(index)
                } else {
                    None
                }
            }
            _ => None,
        }?;
    }

    Some(current)
}

/// Set a nested JSON value using a dot-separated path
pub fn set_nested(value: &mut Value, path: &str, new_value: Value) -> Result<()> {
    let parts: Vec<&str> = path.split('.').collect();
    let mut current = value;

    for (i, part) in parts.iter().enumerate() {
        if i == parts.len() - 1 {
            // Last part - set the value
            match current {
                Value::Object(map) => {
                    map.insert(part.to_string(), new_value.clone());
                }
                Value::Array(arr) => {
                    if let Ok(index) = part.parse::<usize>() {
                        if index < arr.len() {
                            arr[index] = new_value.clone();
                        } else {
                            anyhow::bail!("Array index out of bounds: {}", index);
                        }
                    } else {
                        anyhow::bail!("Invalid array index: {}", part);
                    }
                }
                _ => anyhow::bail!("Cannot set value on non-object/array"),
            }
        } else {
            // Navigate to the next level
            current = match current {
                Value::Object(map) => map
                    .entry(part.to_string())
                    .or_insert_with(|| Value::Object(serde_json::Map::new())),
                Value::Array(arr) => {
                    if let Ok(index) = part.parse::<usize>() {
                        if index < arr.len() {
                            &mut arr[index]
                        } else {
                            anyhow::bail!("Array index out of bounds: {}", index);
                        }
                    } else {
                        anyhow::bail!("Invalid array index: {}", part);
                    }
                }
                _ => anyhow::bail!("Cannot navigate through non-object/array"),
            };
        }
    }

    Ok(())
}

/// Merge two JSON values, with the second taking precedence
pub fn merge_json(base: &Value, override_: &Value) -> Value {
    match (base, override_) {
        (Value::Object(base_map), Value::Object(override_map)) => {
            let mut result = base_map.clone();
            for (key, value) in override_map {
                if let Some(base_value) = base_map.get(key) {
                    result.insert(key.clone(), merge_json(base_value, value));
                } else {
                    result.insert(key.clone(), value.clone());
                }
            }
            Value::Object(result)
        }
        (_, override_val) => override_val.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_nested() {
        let json: Value = serde_json::json!({
            "foo": {
                "bar": {
                    "baz": 42
                }
            },
            "arr": [1, 2, 3]
        });

        assert_eq!(
            get_nested(&json, "foo.bar.baz"),
            Some(&Value::Number(42.into()))
        );
        assert_eq!(get_nested(&json, "arr.1"), Some(&Value::Number(2.into())));
        assert_eq!(get_nested(&json, "foo.missing"), None);
    }

    #[test]
    fn test_set_nested() {
        let mut json: Value = serde_json::json!({
            "foo": {
                "bar": 1
            }
        });

        set_nested(&mut json, "foo.bar", serde_json::json!(2)).unwrap();
        assert_eq!(json["foo"]["bar"], 2);

        set_nested(&mut json, "foo.new", serde_json::json!(3)).unwrap();
        assert_eq!(json["foo"]["new"], 3);
    }

    #[test]
    fn test_merge_json() {
        let base = serde_json::json!({
            "foo": 1,
            "bar": {
                "baz": 2
            }
        });

        let override_ = serde_json::json!({
            "bar": {
                "baz": 3,
                "qux": 4
            },
            "new": 5
        });

        let merged = merge_json(&base, &override_);
        assert_eq!(merged["foo"], 1);
        assert_eq!(merged["bar"]["baz"], 3);
        assert_eq!(merged["bar"]["qux"], 4);
        assert_eq!(merged["new"], 5);
    }
}
