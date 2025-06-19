use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub resources_dir: PathBuf,
    pub preview_dir: PathBuf,
    pub ocr_cache_dir: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 8080,
            resources_dir: PathBuf::from("./resources"),
            preview_dir: PathBuf::from("./resources/.preview"),
            ocr_cache_dir: PathBuf::from("./resources/.ocr_cache"),
        }
    }
}

impl Config {
    pub fn new() -> Self {
        Self::default()
    }
} 