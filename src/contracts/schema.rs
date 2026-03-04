use schemars::{Schema, SchemaGenerator, json_schema};

const JSON_SAFE_INTEGER_MAX: u64 = 9_007_199_254_740_991;

pub fn usize_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "integer",
        "minimum": 0,
        "maximum": JSON_SAFE_INTEGER_MAX
    })
}

pub fn optional_usize_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "integer",
        "minimum": 0,
        "maximum": JSON_SAFE_INTEGER_MAX
    })
}

pub fn u64_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "integer",
        "minimum": 0,
        "maximum": JSON_SAFE_INTEGER_MAX
    })
}

pub fn any_json_value_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "anyOf": [
            {
                "type": "object",
                "additionalProperties": true
            },
            {
                "type": "array",
                "items": {}
            },
            {
                "type": "string"
            },
            {
                "type": "number"
            },
            {
                "type": "boolean"
            }
        ]
    })
}

pub fn sql_params_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "anyOf": [
            {
                "type": "array"
            },
            {
                "type": "object",
                "additionalProperties": true
            }
        ]
    })
}

pub fn import_conflict_mode_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "string",
        "enum": ["none", "ignore", "replace"]
    })
}

pub fn any_object_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "object",
        "additionalProperties": true
    })
}

pub fn any_object_or_null_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "anyOf": [
            {
                "type": "object",
                "additionalProperties": true
            },
            {
                "type": "null"
            }
        ]
    })
}
