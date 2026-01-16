use log::{error, info};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[derive(Clone)]
pub struct FileService {
    resources_dir: PathBuf,
    preview_dir: PathBuf,
    ocr_cache_dir: PathBuf,
}

impl FileService {
    pub fn new(resources_dir: PathBuf, preview_dir: PathBuf, ocr_cache_dir: PathBuf) -> Self {
        Self {
            resources_dir,
            preview_dir,
            ocr_cache_dir,
        }
    }

    pub fn get_preview_dir(&self) -> &PathBuf {
        &self.preview_dir
    }

    pub fn get_resources_dir(&self) -> &PathBuf {
        &self.resources_dir
    }

    pub fn get_pdf_metadata(&self, file: &str) -> Result<HashMap<String, String>, String> {
        let file_path = self.resources_dir.join(file);
        info!("Getting metadata for file: {:?}", file_path);

        let output = Command::new("pdfinfo")
            .arg(&file_path)
            .output()
            .map_err(|e| format!("Failed to execute pdfinfo: {}", e))?;

        if !output.status.success() {
            error!("Failed to get metadata: {:?}", output);
            return Err("Failed to get metadata".to_string());
        }

        let metadata_str = String::from_utf8_lossy(&output.stdout);
        let mut metadata = HashMap::new();

        for line in metadata_str.lines() {
            if let Some((key, value)) = line.split_once(':') {
                metadata.insert(key.trim().to_string(), value.trim().to_string());
            }
        }

        Ok(metadata)
    }

    pub fn generate_preview(&self, file: &str, page: u32) -> Result<PathBuf, String> {
        let file_path = self.resources_dir.join(file);
        let preview_path = self
            .preview_dir
            .join(format!("{}_{}.png", file.replace('/', "_"), page));

        if !preview_path.exists() {
            fs::create_dir_all(&self.preview_dir)
                .map_err(|e| format!("Failed to create preview directory: {}", e))?;

            let output = Command::new("pdftoppm")
                .arg("-png")
                .arg("-singlefile")
                .arg("-f")
                .arg(page.to_string())
                .arg("-l")
                .arg(page.to_string())
                .arg(&file_path)
                .arg(preview_path.with_extension("").to_string_lossy().to_string())
                .output()
                .map_err(|e| format!("Failed to execute pdftoppm: {}", e))?;

            if !output.status.success() {
                error!("Failed to generate PNG for preview: {:?}", output);
                return Err("Failed to generate PNG for preview".to_string());
            }
        }

        Ok(preview_path)
    }

    pub fn save_ocr_cache(
        &self,
        file: &str,
        page: u32,
        provider_id: &str,
        result: serde_json::Value,
    ) -> Result<(), String> {
        let ocr_cache_path = self
            .ocr_cache_dir
            .join(format!("{}_{}.ocr_cache", file.replace('/', "_"), page));

        let ocr_cache_json = serde_json::json!([
            {
                "provider": provider_id,
                "payload": result
            }
        ]);

        fs::create_dir_all(&self.ocr_cache_dir)
            .map_err(|e| format!("Failed to create OCR cache directory: {}", e))?;

        fs::write(
            &ocr_cache_path,
            serde_json::to_string_pretty(&ocr_cache_json)
                .map_err(|e| format!("Failed to serialize OCR cache: {}", e))?,
        )
        .map_err(|e| format!("Failed to write OCR cache: {}", e))
    }

    pub fn get_ocr_cache(&self, file: &str, page: u32) -> Option<String> {
        let ocr_cache_path = self
            .ocr_cache_dir
            .join(format!("{}_{}.ocr_cache", file.replace('/', "_"), page));
        fs::read_to_string(&ocr_cache_path).ok()
    }
}
