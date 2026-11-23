//! Validate the closed native schemas before asking for approval or touching
//! a domain service. Domain code still owns semantic validation and authority.
use serde_json::Value;

pub(super) fn validate(schema: &Value, value: &Value) -> Result<(), String> {
    if let Some(types) = schema.get("type") {
        let matches = |kind: &str| match kind {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "integer" => value.is_i64() || value.is_u64(),
            "number" => value.is_number(),
            "boolean" => value.is_boolean(),
            "null" => value.is_null(),
            _ => false,
        };
        let valid = match types {
            Value::String(kind) => matches(kind),
            Value::Array(kinds) => kinds.iter().filter_map(Value::as_str).any(matches),
            _ => false,
        };
        if !valid {
            return Err("An argument has the wrong type.".into());
        }
    }
    if let Some(options) = schema.get("enum").and_then(Value::as_array) {
        if !options.contains(value) {
            return Err("An argument has an unsupported value.".into());
        }
    }
    if let Some(object) = value.as_object() {
        if let Some(required) = schema.get("required").and_then(Value::as_array) {
            if required
                .iter()
                .filter_map(Value::as_str)
                .any(|key| !object.contains_key(key))
            {
                return Err("A required argument is missing.".into());
            }
        }
        let properties = schema.get("properties").and_then(Value::as_object);
        for (key, value) in object {
            if let Some(field) = properties.and_then(|properties| properties.get(key)) {
                validate(field, value).map_err(|error| format!("{key}: {error}"))?;
            } else if schema.get("additionalProperties") == Some(&Value::Bool(false)) {
                return Err(format!("Unexpected argument: {key}."));
            }
        }
    }
    if let Some(text) = value.as_str() {
        let length = text.chars().count() as u64;
        if schema
            .get("minLength")
            .and_then(Value::as_u64)
            .is_some_and(|min| length < min)
            || schema
                .get("maxLength")
                .and_then(Value::as_u64)
                .is_some_and(|max| length > max)
        {
            return Err("An argument's text length is out of range.".into());
        }
    }
    if let Some(number) = value.as_f64() {
        if schema
            .get("minimum")
            .and_then(Value::as_f64)
            .is_some_and(|min| number < min)
            || schema
                .get("maximum")
                .and_then(Value::as_f64)
                .is_some_and(|max| number > max)
        {
            return Err("A numeric argument is out of range.".into());
        }
    }
    if let Some(items) = value.as_array() {
        if schema
            .get("maxItems")
            .and_then(Value::as_u64)
            .is_some_and(|max| items.len() as u64 > max)
            || schema
                .get("minItems")
                .and_then(Value::as_u64)
                .is_some_and(|min| (items.len() as u64) < min)
        {
            return Err("An argument's item count is out of range.".into());
        }
        if let Some(item_schema) = schema.get("items") {
            for item in items {
                validate(item_schema, item)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn closed_nested_contract_rejects_profile_passthrough_and_wrong_types() {
        let schema = json!({"type":"object","required":["id","patch"],"additionalProperties":false,"properties":{
            "id":{"type":"string","minLength":1},
            "patch":{"type":"object","additionalProperties":false,"properties":{"content":{"type":["string","null"],"maxLength":2},"limit":{"type":"integer","minimum":1,"maximum":100}}}
        }});
        assert!(validate(
            &schema,
            &json!({"id":"a","patch":{"content":"á字","limit":100}})
        )
        .is_ok());
        assert!(validate(&schema, &json!({"id":"a","patch":{"content":null}})).is_ok());
        for args in [
            json!({"id":"a","patch":{},"profile":"other"}),
            json!({"id":"a","patch":{"sql":"DELETE"}}),
            json!({"id":"a","patch":{"limit":1.5}}),
            json!({"id":"a","patch":{"limit":101}}),
            json!({"id":"a","patch":{"content":"abc"}}),
            json!({"patch":{}}),
            json!({"id":"","patch":{}}),
        ] {
            assert!(validate(&schema, &args).is_err(), "{args}");
        }
    }
}
