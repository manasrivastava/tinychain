use std::fmt;

pub type TCResult<T> = Result<T, TCError>;

/// The category of a `TCError`.
#[derive(Clone, Copy, Eq, PartialEq)]
pub enum ErrorType {
    BadRequest,
    Conflict,
    Forbidden,
    Internal,
    MethodNotAllowed,
    NotFound,
    NotImplemented,
    Timeout,
    Unauthorized,
}

impl fmt::Debug for ErrorType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for ErrorType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::BadRequest => f.write_str("bad request"),
            Self::Conflict => f.write_str("conflict"),
            Self::Forbidden => f.write_str("forbidden"),
            Self::Internal => f.write_str("internal error"),
            Self::MethodNotAllowed => f.write_str("method not allowed"),
            Self::NotFound => f.write_str("not found"),
            Self::NotImplemented => f.write_str("not implemented"),
            Self::Timeout => f.write_str("request timeout"),
            Self::Unauthorized => f.write_str("unauthorized"),
        }
    }
}

/// A general error description.
pub struct TCError {
    code: ErrorType,
    message: String,
}

impl TCError {
    /// Error indicating that the request is badly-constructed or nonsensical.
    pub fn bad_request<M: fmt::Display, I: fmt::Display>(message: M, cause: I) -> Self {
        Self {
            code: ErrorType::Internal,
            message: format!("{}: {}", message, cause),
        }
    }

    /// Error indicating that the request depends on a resource which is exclusively locked
    /// by another request.
    pub fn conflict() -> Self {
        Self {
            code: ErrorType::Conflict,
            message: String::default(),
        }
    }

    /// Error indicating that the request actor's credentials do not authorize access to some
    /// request dependencies.
    pub fn forbidden<M: fmt::Display, I: fmt::Display>(message: M, id: I) -> Self {
        Self {
            code: ErrorType::Forbidden,
            message: format!("{}: {}", message, id),
        }
    }

    /// A truly unexpected error, for which the calling application cannot define any specific
    /// handling behavior.
    pub fn internal<I: fmt::Display>(info: I) -> Self {
        Self {
            code: ErrorType::Internal,
            message: info.to_string(),
        }
    }

    /// Error indicating that the requested resource exists but does not support the request method.
    pub fn method_not_allowed<I: fmt::Display>(info: I) -> Self {
        Self {
            code: ErrorType::MethodNotAllowed,
            message: info.to_string(),
        }
    }

    /// Error indicating that the requested resource does not exist at the specified location.
    pub fn not_found<I: fmt::Display>(locator: I) -> Self {
        Self {
            code: ErrorType::NotFound,
            message: locator.to_string(),
        }
    }

    /// Error indicating that a required feature is not yet implemented.
    pub fn not_implemented<F: fmt::Display>(feature: F) -> Self {
        Self {
            code: ErrorType::NotImplemented,
            message: feature.to_string(),
        }
    }

    /// Error indicating that the request failed to complete in the allotted time.
    pub fn timeout<I: fmt::Display>(info: I) -> Self {
        Self {
            code: ErrorType::Timeout,
            message: info.to_string(),
        }
    }

    /// Error indicating that the user's credentials are missing or nonsensical.
    pub fn unauthorized<I: fmt::Display>(info: I) -> Self {
        Self {
            code: ErrorType::Unauthorized,
            message: format!("invalid credentials: {}", info),
        }
    }

    pub fn code(&self) -> ErrorType {
        self.code
    }

    pub fn message(&'_ self) -> &'_ str {
        &self.message
    }
}

impl std::error::Error for TCError {}

impl fmt::Debug for TCError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for TCError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
