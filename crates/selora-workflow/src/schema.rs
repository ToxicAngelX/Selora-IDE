//! JSON-schema-ish validation of `ask` results (ARCHITECTURE.md §18.2).
//!
//! The script declares the shape it expects:
//!
//! ```js
//! const review = agent("reviewer").ask(
//!   {
//!     type: "object",
//!     required: ["findings"],
//!     properties: {
//!       findings: {
//!         type: "array",
//!         items: {
//!           type: "object",
//!           required: ["path", "line", "summary"],
//!           properties: {
//!             path: { type: "string" },
//!             line: { type: "integer" },
//!             summary: { type: "string" }
//!           }
//!         }
//!       }
//!     }
//!   },
//!   "Review src/ for panics"
//! );
//! ```
//!
//! The engine validates the model's answer against that schema **at the
//! boundary**. A mismatch is never coerced: it fails the run with
//! [`crate::WorkflowError::SchemaMismatch`], naming the offending field
//! (`$.findings[0].line`). This is the "model generates, code gates" rule
//! applied to structured results.
//!
//! Supported keywords: `type` (`object`, `array`, `string`, `number`,
//! `integer`, `boolean`, `null`, `any`; a string or an array of strings),
//! `properties`, `required`, `items`, `enum`, `nullable`. Every other keyword
//! (`description`, `title`, …) is documentation and ignored — unknown keywords
//! never reject a value, because the facade is what has to stay narrow, not
//! the schema dialect.

use std::collections::BTreeMap;
use std::fmt;

use serde_json::Value;

/// A validation failure: which field, what was expected, what arrived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaError {
    /// JSON-path-ish location (`$`, `$.findings[0].line`, …).
    pub field: String,
    /// What the schema declared.
    pub expected: String,
    /// What the value actually was.
    pub actual: String,
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "field `{}`: expected {}, got {}",
            self.field, self.expected, self.actual
        )
    }
}

impl std::error::Error for SchemaError {}

/// One JSON type name, plus `any`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeName {
    /// Any JSON object.
    Object,
    /// Any JSON array.
    Array,
    /// Any JSON string.
    String,
    /// Any JSON number.
    Number,
    /// A number with no fractional part.
    Integer,
    /// `true` / `false`.
    Boolean,
    /// `null`.
    Null,
    /// No constraint.
    Any,
}

impl TypeName {
    fn parse(name: &str) -> Result<Self, String> {
        match name {
            "object" => Ok(Self::Object),
            "array" => Ok(Self::Array),
            "string" => Ok(Self::String),
            "number" => Ok(Self::Number),
            "integer" => Ok(Self::Integer),
            "boolean" => Ok(Self::Boolean),
            "null" => Ok(Self::Null),
            "any" => Ok(Self::Any),
            other => Err(format!(
                "unknown `type` `{other}` (use object, array, string, number, integer, boolean, null or any)"
            )),
        }
    }

    fn matches(self, value: &Value) -> bool {
        match self {
            Self::Object => value.is_object(),
            Self::Array => value.is_array(),
            Self::String => value.is_string(),
            Self::Number => value.is_number(),
            Self::Integer => match value {
                Value::Number(number) => {
                    number.as_i64().is_some() || number.as_u64().is_some() || {
                        // A JSON `2.0` is an integer value even if serde keeps it
                        // as f64. Accepting it is not a coercion: 2.0 == 2.
                        number
                            .as_f64()
                            .is_some_and(|float| float.is_finite() && float.fract() == 0.0)
                    }
                }
                _ => false,
            },
            Self::Boolean => value.is_boolean(),
            Self::Null => value.is_null(),
            Self::Any => true,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Object => "object",
            Self::Array => "array",
            Self::String => "string",
            Self::Number => "number",
            Self::Integer => "integer",
            Self::Boolean => "boolean",
            Self::Null => "null",
            Self::Any => "any",
        }
    }
}

/// A parsed schema. Borrows nothing; safe to keep across a run for the journal.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Schema {
    /// Accepted types. Empty means "no type constraint".
    pub types: Vec<TypeName>,
    /// Per-property schemas (validated when the property is present).
    pub properties: BTreeMap<String, Schema>,
    /// Properties that must be present.
    pub required: Vec<String>,
    /// Element schema for arrays.
    pub items: Option<Box<Schema>>,
    /// Closed set of allowed values.
    pub enum_values: Option<Vec<Value>>,
    /// Whether `null` is additionally accepted.
    pub nullable: bool,
}

impl Schema {
    /// Parse the JSON form the script passed as `ask`'s first argument.
    ///
    /// # Errors
    /// A human-readable message when the schema itself is malformed. The
    /// caller turns that into [`crate::WorkflowError::InvalidSchema`].
    pub fn parse(value: &Value) -> Result<Self, String> {
        let object = value
            .as_object()
            .ok_or_else(|| "the `type` argument must be an object literal".to_owned())?;

        let mut types = Vec::new();
        match object.get("type") {
            None => {}
            Some(Value::String(name)) => types.push(TypeName::parse(name)?),
            Some(Value::Array(names)) => {
                for name in names {
                    let name = name
                        .as_str()
                        .ok_or_else(|| "`type` array entries must be strings".to_owned())?;
                    types.push(TypeName::parse(name)?);
                }
            }
            Some(_) => {
                return Err("`type` must be a string or an array of strings".to_owned());
            }
        }

        let mut properties = BTreeMap::new();
        match object.get("properties") {
            None => {}
            Some(Value::Object(props)) => {
                for (key, child) in props {
                    properties.insert(key.clone(), Self::parse(child)?);
                }
            }
            Some(_) => return Err("`properties` must be an object".to_owned()),
        }

        let mut required = Vec::new();
        match object.get("required") {
            None => {}
            Some(Value::Array(names)) => {
                for name in names {
                    let name = name
                        .as_str()
                        .ok_or_else(|| "`required` entries must be strings".to_owned())?;
                    required.push(name.to_owned());
                }
            }
            Some(_) => return Err("`required` must be an array of strings".to_owned()),
        }

        let items = match object.get("items") {
            None => None,
            Some(child) => Some(Box::new(Self::parse(child)?)),
        };

        let enum_values = match object.get("enum") {
            None => None,
            Some(Value::Array(values)) => Some(values.clone()),
            Some(_) => return Err("`enum` must be an array".to_owned()),
        };

        let nullable = match object.get("nullable") {
            None => false,
            Some(Value::Bool(flag)) => *flag,
            Some(_) => return Err("`nullable` must be a boolean".to_owned()),
        };

        Ok(Self {
            types,
            properties,
            required,
            items,
            enum_values,
            nullable,
        })
    }

    /// Validate a value. The returned error names the first offending field.
    ///
    /// # Errors
    /// [`SchemaError`] at the first mismatch, in document order.
    pub fn validate(&self, value: &Value) -> Result<(), SchemaError> {
        self.validate_at(value, "$")
    }

    fn validate_at(&self, value: &Value, path: &str) -> Result<(), SchemaError> {
        if self.nullable && value.is_null() {
            return Ok(());
        }

        if !self.types.is_empty() && !self.types.iter().any(|ty| ty.matches(value)) {
            return Err(SchemaError {
                field: path.to_owned(),
                expected: self.type_label(),
                actual: describe(value).to_owned(),
            });
        }

        if let Some(allowed) = &self.enum_values {
            if !allowed.iter().any(|candidate| candidate == value) {
                return Err(SchemaError {
                    field: path.to_owned(),
                    expected: format!(
                        "one of {}",
                        serde_json::to_string(allowed).unwrap_or_else(|_| "[]".to_owned())
                    ),
                    actual: describe(value).to_owned(),
                });
            }
        }

        if let Some(object) = value.as_object() {
            for name in &self.required {
                if !object.contains_key(name) {
                    return Err(SchemaError {
                        field: child_path(path, name),
                        expected: "required property".to_owned(),
                        actual: "missing".to_owned(),
                    });
                }
            }
            for (name, schema) in &self.properties {
                if let Some(child) = object.get(name) {
                    schema.validate_at(child, &child_path(path, name))?;
                }
            }
        }

        if let (Some(items), Some(array)) = (&self.items, value.as_array()) {
            for (index, child) in array.iter().enumerate() {
                items.validate_at(child, &format!("{path}[{index}]"))?;
            }
        }

        Ok(())
    }

    fn type_label(&self) -> String {
        self.types
            .iter()
            .map(|ty| ty.label())
            .collect::<Vec<_>>()
            .join(" or ")
    }
}

fn child_path(parent: &str, name: &str) -> String {
    let simple = !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '$');
    if simple {
        format!("{parent}.{name}")
    } else {
        format!("{parent}[{}]", Value::String(name.to_owned()))
    }
}

fn describe(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parsed(schema: Value) -> Schema {
        Schema::parse(&schema).expect("schema parses")
    }

    #[test]
    fn accepts_a_conforming_value() {
        let schema = parsed(json!({
            "type": "object",
            "required": ["findings"],
            "properties": {
                "findings": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "required": ["path", "line"],
                        "properties": {
                            "path": { "type": "string" },
                            "line": { "type": "integer" }
                        }
                    }
                }
            }
        }));
        let value = json!({ "findings": [{ "path": "src/lib.rs", "line": 42 }] });
        assert_eq!(schema.validate(&value), Ok(()));
    }

    #[test]
    fn names_the_offending_nested_field() {
        let schema = parsed(json!({
            "type": "object",
            "properties": {
                "findings": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": { "line": { "type": "integer" } }
                    }
                }
            }
        }));
        let value = json!({ "findings": [{ "line": "twelve" }] });
        let error = schema.validate(&value).expect_err("mismatch is rejected");
        assert_eq!(error.field, "$.findings[0].line");
        assert_eq!(error.expected, "integer");
        assert_eq!(error.actual, "string");
    }

    #[test]
    fn never_coerces_numbers_into_strings_or_back() {
        let schema = parsed(json!({ "type": "string" }));
        assert!(schema.validate(&json!(7)).is_err());
        let schema = parsed(json!({ "type": "integer" }));
        assert!(schema.validate(&json!("7")).is_err());
        assert!(schema.validate(&json!(1.5)).is_err());
        assert!(schema.validate(&json!(7.0)).is_ok());
    }

    #[test]
    fn reports_missing_required_property_by_name() {
        let schema = parsed(json!({ "type": "object", "required": ["path"] }));
        let error = schema.validate(&json!({})).expect_err("missing field");
        assert_eq!(error.field, "$.path");
        assert_eq!(error.actual, "missing");
    }

    #[test]
    fn enum_and_nullable() {
        let schema = parsed(json!({ "enum": ["low", "high"] }));
        assert!(schema.validate(&json!("low")).is_ok());
        assert_eq!(
            schema
                .validate(&json!("medium"))
                .expect_err("not in enum")
                .expected,
            "one of [\"low\",\"high\"]"
        );

        let schema = parsed(json!({ "type": "string", "nullable": true }));
        assert!(schema.validate(&Value::Null).is_ok());
    }

    #[test]
    fn rejects_malformed_schemas() {
        assert!(Schema::parse(&json!("not an object")).is_err());
        assert!(Schema::parse(&json!({ "type": "tuple" })).is_err());
        assert!(Schema::parse(&json!({ "required": "path" })).is_err());
        assert!(Schema::parse(&json!({ "items": [] })).is_err());
    }

    #[test]
    fn empty_schema_accepts_anything() {
        let schema = parsed(json!({ "description": "free-form" }));
        assert!(schema.validate(&json!({ "anything": [1, 2, 3] })).is_ok());
    }
}
