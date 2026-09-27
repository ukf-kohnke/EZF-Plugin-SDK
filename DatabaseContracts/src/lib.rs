#![allow(non_snake_case)]
/// # Database Contracts Library
///
/// This library provides core data structures and enums for database operations,
/// enabling type-safe communication between database clients and services.
///
/// ## Key Types
///
/// - [`DatabaseType`]: Enumeration of supported database column types
/// - [`DatabaseValue`]: Tagged enum representing actual database values with type information
/// - [`DatabaseRequest`]: Enum for query and execute operations
/// - [`DatabaseResponse`]: Enum for query results and execution confirmations
/// - [`DatabaseError`]: Error type with categorized error kinds
///
/// ## Usage Example
///
/// ```ignore
/// use database_contracts::{DatabaseRequest, QueryRequest, DatabaseParameter, ResultColumn, DatabaseType};
///
/// let query = DatabaseRequest::Query(QueryRequest {
///     statement: "SELECT * FROM users WHERE id = ?".to_string(),
///     parameters: vec![],
///     columns: vec![ResultColumn::new("id", DatabaseType::I32)],
/// });
/// ```
///
/// ## Serialization
///
/// All types implement [`Serialize`] and [`Deserialize`] for seamless JSON/binary
/// serialization. The enums use tagged formats for unambiguous type identification.
use chrono::{ NaiveDate, NaiveDateTime, NaiveTime };
use serde::{ Deserialize, Serialize };
use std::fmt;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DatabaseType {
    Bool,
    I16,
    I32,
    I64,
    F32,
    F64,
    Text,
    Bytes,
    Uuid,
    Date,
    Time,
    DateTime,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum DatabaseValue {
    Null(DatabaseType),
    Bool(bool),
    I16(i16),
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
    Text(String),
    Bytes(Vec<u8>),
    Uuid(Uuid),
    Date(NaiveDate),
    Time(NaiveTime),
    DateTime(NaiveDateTime),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DatabaseParameter {
    pub value: DatabaseValue,
}

impl DatabaseParameter {
    #[must_use]
    pub fn new(value: DatabaseValue) -> Self {
        Self { value }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResultColumn {
    pub name: String,
    pub data_type: DatabaseType,
}

impl ResultColumn {
    #[must_use]
    pub fn new(name: impl Into<String>, data_type: DatabaseType) -> Self {
        Self { name: name.into(), data_type }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryRequest {
    pub statement: String,
    pub parameters: Vec<DatabaseParameter>,
    pub columns: Vec<ResultColumn>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseExecutionContext {
    /// Optional actor identity associated with a mutating database execution.
    ///
    /// DatabaseContracts stays deliberately agnostic of cache implementations. Consumers such
    /// as database plugins may use this context for post-commit side effects owned by the plugin.
    #[serde(default)]
    pub actor: Option<String>,
}

impl DatabaseExecutionContext {
    #[must_use]
    pub fn for_actor(actor: impl Into<String>) -> Self {
        Self { actor: Some(actor.into()) }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExecuteRequest {
    pub statement: String,
    pub parameters: Vec<DatabaseParameter>,
    #[serde(default)]
    pub context: DatabaseExecutionContext,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum DatabaseRequest {
    Query(QueryRequest),
    Execute(ExecuteRequest),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DatabaseColumn {
    pub name: String,
    pub value: DatabaseValue,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DatabaseRow {
    pub columns: Vec<DatabaseColumn>,
}

impl DatabaseRow {
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&DatabaseValue> {
        self.columns
            .iter()
            .find(|column| column.name == name)
            .map(|column| &column.value)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ResultSet {
    pub rows: Vec<DatabaseRow>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionResult {
    pub rows_affected: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum DatabaseResponse {
    Query(ResultSet),
    Execute(ExecutionResult),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DatabaseErrorKind {
    Unsupported,
    InvalidRequest,
    Connection,
    Query,
    Decode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseError {
    pub kind: DatabaseErrorKind,
    pub message: String,
}

impl DatabaseError {
    #[must_use]
    pub fn new(kind: DatabaseErrorKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into() }
    }

    #[must_use]
    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(DatabaseErrorKind::Unsupported, message)
    }
}

impl fmt::Display for DatabaseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for DatabaseError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_context_can_carry_an_actor_without_database_specific_knowledge() {
        let context = DatabaseExecutionContext::for_actor("user-1");

        assert_eq!(context.actor.as_deref(), Some("user-1"));
        assert_eq!(DatabaseExecutionContext::default().actor, None);
    }

    #[test]
    fn execute_request_defaults_to_an_empty_execution_context() {
        let request = ExecuteRequest {
            statement: "SELECT 1".to_owned(),
            parameters: Vec::new(),
            ..Default::default()
        };

        assert_eq!(request.context, DatabaseExecutionContext::default());
    }

    #[test]
    fn row_lookup_preserves_the_typed_value() {
        let row = DatabaseRow {
            columns: vec![DatabaseColumn {
                name: "payload".to_owned(),
                value: DatabaseValue::Text("{}".to_owned()),
            }],
        };

        assert_eq!(row.get("payload"), Some(&DatabaseValue::Text("{}".to_owned())));
        assert!(row.get("missing").is_none());
    }
}
