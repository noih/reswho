use serde::{Deserialize, Serialize};
use std::{fmt, io};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub operation: String,
    pub message: String,
    pub os_error: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<FailureReason>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureReason {
    CliNotFound,
    EndpointNotFound,
    PermissionDenied,
    ConnectionRefused,
    DaemonError,
    Timeout,
    Canceled,
    Unknown,
}

impl FailureReason {
    pub fn code(self) -> &'static str {
        match self {
            Self::CliNotFound => "cli_not_found",
            Self::EndpointNotFound => "endpoint_not_found",
            Self::PermissionDenied => "permission_denied",
            Self::ConnectionRefused => "connection_refused",
            Self::DaemonError => "daemon_error",
            Self::Timeout => "timeout",
            Self::Canceled => "canceled",
            Self::Unknown => "unknown",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Self::CliNotFound => "runtime CLI not found on PATH; install it or fix PATH",
            Self::EndpointNotFound => {
                "runtime endpoint or service not found; start the runtime or check the selected context/host"
            }
            Self::PermissionDenied => {
                "endpoint socket access denied; allow socket access (sandbox or file mode)"
            }
            Self::ConnectionRefused => {
                "endpoint is not accepting connections; the runtime may be stopped or busy, or the socket is stale"
            }
            Self::DaemonError => {
                "runtime returned an error or unsupported data; check runtime version and health"
            }
            Self::Timeout => "runtime did not answer before the collection deadline; retry later",
            Self::Canceled => "collection was canceled before the runtime answered",
            Self::Unknown => "cause not determined; run the runtime CLI directly to diagnose",
        }
    }
}

fn suffix(reason: FailureReason) -> String {
    format!(" · reason {}: {}", reason.code(), reason.hint())
}

impl Diagnostic {
    pub fn new(
        code: &'static str,
        operation: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            operation: operation.into(),
            message: message.into(),
            os_error: None,
            reason: None,
        }
    }

    pub fn failure(
        code: &'static str,
        operation: impl Into<String>,
        message: impl std::fmt::Display,
        reason: FailureReason,
        os_error: Option<i32>,
    ) -> Self {
        Self {
            code: code.into(),
            operation: operation.into(),
            message: format!("{message}{}", suffix(reason)),
            os_error,
            reason: Some(reason),
        }
    }

    pub fn refine(&mut self, reason: FailureReason, os_error: Option<i32>) {
        let Some(previous) = self.reason else {
            return;
        };
        if let Some(base) = self.message.strip_suffix(&suffix(previous)) {
            self.message = format!("{base}{}", suffix(reason));
        }
        self.reason = Some(reason);
        self.os_error = os_error.or(self.os_error);
    }

    pub fn io(operation: &str, error: &io::Error) -> Self {
        let code = match error.kind() {
            io::ErrorKind::PermissionDenied => "permission_denied",
            io::ErrorKind::NotFound => "not_found",
            _ => "io_error",
        };
        Self {
            code: code.into(),
            operation: operation.into(),
            message: format!("{operation}: {error}"),
            os_error: error.raw_os_error(),
            reason: None,
        }
    }

    pub fn is_process_details(&self) -> bool {
        self.operation.starts_with("extensions.") && self.operation.ends_with(".details")
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
