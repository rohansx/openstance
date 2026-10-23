//! Errors as the shared `Error` JSON (interfaces.md 1.11) with the matching
//! HTTP status. Core errors map through `ErrorCode::http_status`; the HTTP
//! layer adds four transport-only codes (`forbidden` 403, `misdirected` 421,
//! `payload_too_large` 413, `method_not_allowed` 405) that no other surface
//! can produce.

use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use openstance_core::json::{ErrorResponse, V};
use serde_json::{json, Value};

/// An error response.
#[derive(Debug, Clone, PartialEq)]
pub struct ApiError {
    /// HTTP status.
    pub status: StatusCode,
    /// `{"v": 1, "error": {...}}`.
    pub body: Value,
}

impl ApiError {
    fn transport(status: StatusCode, code: &str, message: impl Into<String>) -> ApiError {
        ApiError {
            status,
            body: json!({ "v": V, "error": { "code": code, "message": message.into(), "candidates": [] } }),
        }
    }

    /// From a core error.
    pub fn core(e: &openstance_core::Error) -> ApiError {
        let resp = ErrorResponse::from(e);
        if resp.error.code == openstance_core::json::ErrorCode::Internal {
            tracing::error!(error = %e, "internal error");
        }
        Self::from_response(&resp)
    }

    /// From an error envelope (an MCP tool's structured error, say).
    pub fn from_response(resp: &ErrorResponse) -> ApiError {
        let status = StatusCode::from_u16(resp.error.code.http_status())
            .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        ApiError {
            status,
            body: serde_json::to_value(resp).unwrap_or(Value::Null),
        }
    }

    /// From a JSON envelope that is already shaped like an error.
    pub fn from_envelope(v: Value) -> ApiError {
        match serde_json::from_value::<ErrorResponse>(v.clone()) {
            Ok(r) => Self::from_response(&r),
            Err(_) => Self::internal("malformed error"),
        }
    }

    /// 400 `bad_request`.
    pub fn bad_request(message: impl Into<String>) -> ApiError {
        Self::core(&openstance_core::Error::BadRequest(message.into()))
    }

    /// 404 `not_found`.
    pub fn not_found(what: impl Into<String>) -> ApiError {
        Self::core(&openstance_core::Error::NotFound(what.into()))
    }

    /// 500 `internal`. The message is safe to show; details go to the log.
    pub fn internal(message: impl Into<String>) -> ApiError {
        Self::transport(StatusCode::INTERNAL_SERVER_ERROR, "internal", message)
    }

    /// 401 `unauthorized`.
    pub fn unauthorized(message: impl Into<String>) -> ApiError {
        Self::transport(StatusCode::UNAUTHORIZED, "unauthorized", message)
    }

    /// 403 `forbidden` (a cross-origin request from an origin not listed in
    /// `serve.cors_origins`).
    pub fn forbidden(message: impl Into<String>) -> ApiError {
        Self::transport(StatusCode::FORBIDDEN, "forbidden", message)
    }

    /// 421 `misdirected` (Host header check, DNS-rebinding defence).
    pub fn misdirected(message: impl Into<String>) -> ApiError {
        Self::transport(StatusCode::MISDIRECTED_REQUEST, "misdirected", message)
    }

    /// 413 `payload_too_large`.
    pub fn too_large(message: impl Into<String>) -> ApiError {
        Self::transport(StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large", message)
    }

    /// 405 `method_not_allowed`.
    pub fn method_not_allowed() -> ApiError {
        Self::transport(
            StatusCode::METHOD_NOT_ALLOWED,
            "method_not_allowed",
            "method not allowed on this route",
        )
    }
}

impl From<openstance_core::Error> for ApiError {
    fn from(e: openstance_core::Error) -> Self {
        ApiError::core(&e)
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(e: serde_json::Error) -> Self {
        ApiError::internal(format!("json: {e}"))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut r = (self.status, json_bytes(&self.body)).into_response();
        r.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json; charset=utf-8"),
        );
        if self.status == StatusCode::UNAUTHORIZED {
            r.headers_mut()
                .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        r
    }
}

/// Compact JSON bytes.
pub fn json_bytes(v: &Value) -> Vec<u8> {
    serde_json::to_vec(v).unwrap_or_default()
}

/// A `200 application/json` response of any serializable value.
pub fn ok_json<T: serde::Serialize>(v: &T) -> Result<Response, ApiError> {
    let bytes = serde_json::to_vec(v)?;
    let mut r = (StatusCode::OK, bytes).into_response();
    r.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_errors_keep_their_code_and_status() {
        let e = ApiError::from(openstance_core::Error::Ambiguous {
            reference: "Sam".into(),
            candidates: vec![(3, "Sam Ito".into()), (9, "Sam Park".into())],
        });
        assert_eq!(e.status, StatusCode::CONFLICT);
        assert_eq!(e.body["v"], 1);
        assert_eq!(e.body["error"]["code"], "ambiguous");
        assert_eq!(e.body["error"]["candidates"][1]["id"], 9);
        assert_eq!(
            ApiError::not_found("person 7").status,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            ApiError::bad_request("x").body["error"]["code"],
            "bad_request"
        );
    }

    #[test]
    fn transport_codes() {
        assert_eq!(ApiError::unauthorized("x").status, StatusCode::UNAUTHORIZED);
        assert_eq!(ApiError::forbidden("x").body["error"]["code"], "forbidden");
        assert_eq!(ApiError::misdirected("x").status.as_u16(), 421);
        assert_eq!(ApiError::too_large("x").status.as_u16(), 413);
    }

    #[test]
    fn envelope_round_trip() {
        let e = ApiError::not_found("view x");
        let back = ApiError::from_envelope(e.body.clone());
        assert_eq!(back, e);
    }
}
