use schemars::{Schema, SchemaGenerator, json_schema};

const JSON_SAFE_INTEGER_MAX: u64 = 9_007_199_254_740_991;
const JSON_SAFE_INTEGER_MIN: i64 = -9_007_199_254_740_991;

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
                "type": "array",
                "items": {}
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

pub fn nonnegative_i64_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "integer",
        "minimum": 0,
        "maximum": JSON_SAFE_INTEGER_MAX
    })
}

pub fn optional_nonnegative_i64_schema(_: &mut SchemaGenerator) -> Schema {
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

pub fn i64_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "integer",
        "minimum": JSON_SAFE_INTEGER_MIN,
        "maximum": JSON_SAFE_INTEGER_MAX
    })
}

pub fn number_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "number"
    })
}

#[cfg(test)]
mod tests {
    use schemars::schema_for;
    use serde_json::Value;

    use crate::contracts::{
        queue::{QueueJobSlot, QueuePushData, QueueWaitRequest},
        sql::{
            SqlBatchRequest, SqlBatchResult, SqlExecuteData, SqlExecuteRequest, SqlQueryRequest,
        },
    };

    fn assert_no_array_schema_missing_items(value: &Value) {
        walk_schema(value, &mut |node| {
            let is_array = node
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|value| value == "array");
            if is_array {
                assert!(
                    node.get("items").is_some(),
                    "array schema missing items: {node:?}"
                );
            }
        });
    }

    fn assert_no_unsupported_formats(value: &Value) {
        walk_schema(value, &mut |node| {
            if let Some(format) = node.get("format").and_then(Value::as_str) {
                assert!(
                    !matches!(format, "int64" | "uint64" | "double"),
                    "unsupported format {format}: {node:?}"
                );
            }
        });
    }

    fn walk_schema(value: &Value, visit: &mut dyn FnMut(&serde_json::Map<String, Value>)) {
        match value {
            Value::Object(map) => {
                visit(map);
                for child in map.values() {
                    walk_schema(child, visit);
                }
            }
            Value::Array(items) => {
                for item in items {
                    walk_schema(item, visit);
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }

    fn assert_schema_is_mcp_compatible<T: schemars::JsonSchema>() {
        let schema = serde_json::to_value(schema_for!(T)).expect("schema must serialize");
        assert_no_array_schema_missing_items(&schema);
        assert_no_unsupported_formats(&schema);
    }

    #[test]
    fn sql_request_schemas_publish_array_items_and_supported_formats() {
        assert_schema_is_mcp_compatible::<SqlQueryRequest>();
        assert_schema_is_mcp_compatible::<SqlExecuteRequest>();
        assert_schema_is_mcp_compatible::<SqlBatchRequest>();
    }

    #[test]
    fn sql_response_schemas_publish_supported_integer_contracts() {
        assert_schema_is_mcp_compatible::<SqlExecuteData>();
        assert_schema_is_mcp_compatible::<SqlBatchResult>();
    }

    #[test]
    fn queue_schemas_publish_supported_integer_contracts() {
        let queue_wait_schema =
            serde_json::to_value(schema_for!(QueueWaitRequest)).expect("schema must serialize");
        assert_no_array_schema_missing_items(&queue_wait_schema);
        assert_no_unsupported_formats(&queue_wait_schema);
        assert_eq!(
            queue_wait_schema.pointer("/properties/after_id/minimum"),
            Some(&Value::from(0))
        );

        assert_schema_is_mcp_compatible::<QueuePushData>();
        assert_schema_is_mcp_compatible::<QueueJobSlot>();

        let queue_job_slot_schema =
            serde_json::to_value(schema_for!(QueueJobSlot)).expect("schema must serialize");
        assert_eq!(
            queue_job_slot_schema.pointer("/properties/id/anyOf/0/minimum"),
            Some(&Value::from(0))
        );
        assert_eq!(
            queue_job_slot_schema.pointer("/properties/id/anyOf/1/type"),
            Some(&Value::from("null"))
        );
    }

    #[cfg(feature = "vector")]
    #[test]
    fn vector_search_schema_uses_plain_numbers() {
        use crate::contracts::vector::{VectorMatch, VectorSearchData};

        assert_schema_is_mcp_compatible::<VectorSearchData>();

        let match_schema =
            serde_json::to_value(schema_for!(VectorMatch)).expect("schema must serialize");
        assert_eq!(
            match_schema.pointer("/required"),
            Some(&Value::from(vec!["id", "distance"]))
        );
    }

    #[test]
    fn sql_optional_rowid_fields_remain_optional() {
        let execute_schema =
            serde_json::to_value(schema_for!(SqlExecuteData)).expect("schema must serialize");
        assert_eq!(
            execute_schema.pointer("/required"),
            Some(&Value::from(vec!["rows_affected"]))
        );

        let batch_schema =
            serde_json::to_value(schema_for!(SqlBatchResult)).expect("schema must serialize");
        assert_eq!(
            batch_schema.pointer("/required"),
            Some(&Value::from(vec!["index", "kind", "rows_affected"]))
        );
    }
}
