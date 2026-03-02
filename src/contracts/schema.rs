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
        "anyOf": [
            {
                "type": "integer",
                "minimum": 0,
                "maximum": JSON_SAFE_INTEGER_MAX
            },
            {
                "type": "null"
            }
        ]
    })
}

pub fn u64_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "integer",
        "minimum": 0,
        "maximum": JSON_SAFE_INTEGER_MAX
    })
}
