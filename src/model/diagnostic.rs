use serde::Serialize;
use std::{fmt, io};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Diagnostic {
    pub code: &'static str,
    pub operation: String,
    pub message: String,
    pub os_error: Option<i32>,
}

impl Diagnostic {
    pub fn new(
        code: &'static str,
        operation: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code,
            operation: operation.into(),
            message: message.into(),
            os_error: None,
        }
    }

    pub fn io(operation: &str, error: &io::Error) -> Self {
        let code = match error.kind() {
            io::ErrorKind::PermissionDenied => "permission_denied",
            io::ErrorKind::NotFound => "not_found",
            _ => "io_error",
        };
        Self {
            code,
            operation: operation.into(),
            message: format!("{operation}: {error}"),
            os_error: error.raw_os_error(),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<String> for Diagnostic {
    fn from(message: String) -> Self {
        Self::new("unavailable", "collection", message)
    }
}

impl From<&str> for Diagnostic {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}
