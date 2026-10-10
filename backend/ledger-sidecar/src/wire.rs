//! JSON-RPC 2.0 envelopes. A message is one line.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
pub struct Request {
    /// Absent in a notification, which gets no answer.
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

/// The codes of JSON-RPC, and one for what went wrong in the application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    ParseError = -32700,
    MethodNotFound = -32601,
    InvalidParams = -32602,
    Failed = -32000,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcError {
    pub code: ErrorCode,
    pub message: String,
}

impl RpcError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn failed(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Failed, message)
    }

    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidParams, message)
    }
}

pub fn response(id: &Value, result: &Result<Value, RpcError>) -> String {
    let message = match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(error) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": error.code as i32, "message": error.message },
        }),
    };
    message.to_string()
}

pub fn notification(method: &str, params: &impl Serialize) -> String {
    json!({ "jsonrpc": "2.0", "method": method, "params": params }).to_string()
}
