use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub resources_dir: PathBuf,
    pub preview_dir: PathBuf,
    pub ocr_cache_dir: PathBuf,
    pub base_url: String,
}

impl Default for Config {
    fn default() -> Self {
        let host = std::env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        let port: u16 = std::env::var("PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(8081);

        Self {
            host: host.clone(),
            port,
            resources_dir: PathBuf::from(
                std::env::var("RESOURCES_DIR").unwrap_or_else(|_| "./resources".to_string()),
            ),
            preview_dir: PathBuf::from(
                std::env::var("PREVIEW_DIR").unwrap_or_else(|_| "./resources/.preview".to_string()),
            ),
            ocr_cache_dir: PathBuf::from(
                std::env::var("OCR_CACHE_DIR")
                    .unwrap_or_else(|_| "./resources/.ocr_cache".to_string()),
            ),
            base_url: std::env::var("BASE_URL")
                .unwrap_or_else(|_| format!("http://{}:{}", host, port)),
        }
    }
}

impl Config {
    pub fn new() -> Self {
        Self::default()
    }
}
