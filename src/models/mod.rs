use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt;

#[derive(Debug)]
pub struct OcrError(pub String);

impl fmt::Display for OcrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "OcrError: {}", self.0)
    }
}

impl Error for OcrError {}

#[derive(Debug, Deserialize)]
pub struct PreviewImageParams {
    pub filename: String,
    pub page: usize,
}

#[derive(Debug, Deserialize)]
pub struct PreviewParams {
    pub file: String,
    pub page: u32,
}

#[derive(Debug, Serialize)]
pub struct OcrResponse {
    pub result: String,
}

#[derive(Debug, Serialize)]
pub struct MetadataResponse {
    pub metadata: std::collections::HashMap<String, String>,
} 