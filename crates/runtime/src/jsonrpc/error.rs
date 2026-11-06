use std::{
    borrow::Cow,
    fmt::{self, Display},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// An error code that JSON-RPC or the protocol defines, or any other a server uses.
///
/// Codes compare by number, so `ServerError(-32801)` equals `ContentModified`, as a client would
/// read it. The enum is non-exhaustive because the protocol adds codes, and a new variant must not
/// break a server that matches on them.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(into = "i64", from = "i64")]
#[non_exhaustive]
pub enum ErrorCode {
    ParseError,
    InvalidRequest,
    MethodNotFound,
    InvalidParams,
    InternalError,
    /// A request before `initialize` was answered (`-32002`).
    ServerNotInitialized,
    /// `-32001`, which the protocol reserves for an error that has no code of its own.
    UnknownErrorCode,
    /// The client cancelled the request (`-32800`).
    RequestCancelled,
    /// The document changed while the request ran, so the answer would describe stale text
    /// (`-32801`).
    ContentModified,
    /// The server cancelled the request, and the client may send it again (`-32802`).
    ServerCancelled,
    /// The request was valid but failed (`-32803`).
    RequestFailed,
    /// A code from the range reserved for implementations, or any other.
    ServerError(i64),
}

impl PartialEq for ErrorCode {
    fn eq(&self, other: &ErrorCode) -> bool {
        self.code() == other.code()
    }
}

impl Eq for ErrorCode {}

impl std::hash::Hash for ErrorCode {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.code().hash(state);
    }
}

impl ErrorCode {
    pub const fn code(&self) -> i64 {
        match self {
            ErrorCode::ParseError => -32700,
            ErrorCode::InvalidRequest => -32600,
            ErrorCode::MethodNotFound => -32601,
            ErrorCode::InvalidParams => -32602,
            ErrorCode::InternalError => -32603,
            ErrorCode::ServerNotInitialized => -32002,
            ErrorCode::UnknownErrorCode => -32001,
            ErrorCode::RequestCancelled => -32800,
            ErrorCode::ContentModified => -32801,
            ErrorCode::ServerCancelled => -32802,
            ErrorCode::RequestFailed => -32803,
            ErrorCode::ServerError(code) => *code,
        }
    }

    pub const fn description(&self) -> &'static str {
        match self {
            ErrorCode::ParseError => "Parse error",
            ErrorCode::InvalidRequest => "Invalid request",
            ErrorCode::MethodNotFound => "Method not found",
            ErrorCode::InvalidParams => "Invalid params",
            ErrorCode::InternalError => "Internal error",
            ErrorCode::ServerNotInitialized => "Server not initialized",
            ErrorCode::UnknownErrorCode => "Unknown error code",
            ErrorCode::RequestCancelled => "Canceled",
            ErrorCode::ContentModified => "Content modified",
            ErrorCode::ServerCancelled => "Server cancelled",
            ErrorCode::RequestFailed => "Request failed",
            ErrorCode::ServerError(_) => "Server error",
        }
    }
}

impl From<i64> for ErrorCode {
    fn from(code: i64) -> ErrorCode {
        match code {
            -32700 => ErrorCode::ParseError,
            -32600 => ErrorCode::InvalidRequest,
            -32601 => ErrorCode::MethodNotFound,
            -32602 => ErrorCode::InvalidParams,
            -32603 => ErrorCode::InternalError,
            -32002 => ErrorCode::ServerNotInitialized,
            -32001 => ErrorCode::UnknownErrorCode,
            -32800 => ErrorCode::RequestCancelled,
            -32801 => ErrorCode::ContentModified,
            -32802 => ErrorCode::ServerCancelled,
            -32803 => ErrorCode::RequestFailed,
            code => ErrorCode::ServerError(code),
        }
    }
}

impl From<ErrorCode> for i64 {
    fn from(code: ErrorCode) -> i64 {
        code.code()
    }
}

impl Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.code(), f)
    }
}

/// The error a request is answered with.
///
/// Unknown fields are ignored when reading one. Rejecting them made the client's whole response
/// unreadable, and the request kept waiting for an answer that had already arrived.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Error {
    pub code: ErrorCode,
    pub message: Cow<'static, str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl Error {
    /// An error with the code's own description as its message.
    pub const fn new(code: ErrorCode) -> Error {
        Error {
            code,
            message: Cow::Borrowed(code.description()),
            data: None,
        }
    }

    pub const fn parse_error() -> Error {
        Error::new(ErrorCode::ParseError)
    }

    pub const fn invalid_request() -> Error {
        Error::new(ErrorCode::InvalidRequest)
    }

    pub const fn method_not_found() -> Error {
        Error::new(ErrorCode::MethodNotFound)
    }

    pub fn invalid_params(message: impl Into<Cow<'static, str>>) -> Error {
        Error {
            code: ErrorCode::InvalidParams,
            message: message.into(),
            data: None,
        }
    }

    pub const fn internal_error() -> Error {
        Error::new(ErrorCode::InternalError)
    }

    pub const fn request_cancelled() -> Error {
        Error::new(ErrorCode::RequestCancelled)
    }

    pub const fn content_modified() -> Error {
        Error::new(ErrorCode::ContentModified)
    }

    /// A request before `initialize`: the protocol's `-32002`.
    pub(crate) const fn not_initialized() -> Error {
        Error {
            code: ErrorCode::ServerNotInitialized,
            message: Cow::Borrowed("Server not initialized"),
            data: None,
        }
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code.description(), self.message)
    }
}

impl std::error::Error for Error {}
