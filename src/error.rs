use actix_web::{HttpResponse, ResponseError};
use std::fmt;

#[derive(Debug)]
pub enum AppError {
    NotFound(String),
    Internal(String),
    BadRequest(String),
    Config(String),
    Ocr(String),
    Io(std::io::Error),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::NotFound(msg) => write!(f, "Not found: {}", msg),
            AppError::Internal(msg) => write!(f, "Internal error: {}", msg),
            AppError::BadRequest(msg) => write!(f, "Bad request: {}", msg),
            AppError::Config(msg) => write!(f, "Configuration error: {}", msg),
            AppError::Ocr(msg) => write!(f, "OCR error: {}", msg),
            AppError::Io(e) => write!(f, "IO error: {}", e),
        }
    }
}

impl std::error::Error for AppError {}

impl ResponseError for AppError {
    fn error_response(&self) -> HttpResponse {
        let body = serde_json::json!({ "error": self.to_string() });
        match self {
            AppError::NotFound(_) => HttpResponse::NotFound().json(body),
            AppError::BadRequest(_) => HttpResponse::BadRequest().json(body),
            _ => HttpResponse::InternalServerError().json(body),
        }
    }
}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        AppError::Io(err)
    }
}

impl From<String> for AppError {
    fn from(msg: String) -> Self {
        AppError::Internal(msg)
    }
}

pub type AppResult<T> = Result<T, AppError>;
