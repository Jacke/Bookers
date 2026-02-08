use crate::config::Config;
use crate::models::OcrError;
use async_trait::async_trait;
use base64::Engine;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// OCR Service for running OCR on images
#[derive(Clone)]
pub struct OcrService {
    preview_dir: PathBuf,
}

impl OcrService {
    pub fn new(preview_dir: PathBuf) -> Self {
        Self { preview_dir }
    }
    
    /// Run OCR on an image file
    pub async fn run_ocr(&self, image_path: &Path, provider: &str) -> anyhow::Result<String> {
        // Check if preview image exists
        if !image_path.exists() {
            return Err(anyhow::anyhow!("Image not found: {:?}", image_path));
        }
        
        // Try to use venv python first
        let python_path = if std::path::Path::new(".venv/bin/python").exists() {
            ".venv/bin/python"
        } else if std::path::Path::new(".venv/bin/python3").exists() {
            ".venv/bin/python3"
        } else {
            "python3"
        };
        
        let output = tokio::task::spawn_blocking({
            let path = image_path.to_path_buf();
            let py = python_path.to_string();
            let prov = provider.to_string();
            move || {
                std::process::Command::new(&py)
                    .arg("ocr.py")
                    .arg(&path)
                    .arg("-p")
                    .arg(&prov)
                    .output()
            }
        })
        .await
        .map_err(|e| anyhow::anyhow!("Task join error: {}", e))?;
        
        let output = output.map_err(|e| anyhow::anyhow!("Failed to run OCR: {}", e))?;
        
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("OCR script error: {}", stderr));
        }
        
        let text = String::from_utf8_lossy(&output.stdout);
        Ok(text.trim().to_string())
    }
}

#[async_trait]
pub trait OcrProvider: Send + Sync {
    async fn extract_text(
        &self,
        image_path: &str,
        file: &str,
        page: u32,
    ) -> Result<(String, Value), OcrError>;
    fn provider_id(&self) -> &'static str;
}

pub struct MistralOcrProvider {
    api_key: String,
    config: Config,
}

impl MistralOcrProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            config: Config::new(),
        }
    }
}

#[async_trait]
impl OcrProvider for MistralOcrProvider {
    async fn extract_text(
        &self,
        image_path: &str,
        file: &str,
        page: u32,
    ) -> Result<(String, Value), OcrError> {
        let image_base64_url = crate::utils::encode_image_to_base64(image_path)
            .map_err(|e| OcrError(format!("Failed to encode image to base64: {}", e)))?;

        let client = reqwest::Client::new();
        let request_body = serde_json::json!({
            "document": {
                "type": "image_url",
                "image_url": image_base64_url
            },
            "include_image_base64": true,
            "model": "mistral-ocr-latest"
        });

        let resp = client
            .post("https://api.mistral.ai/v1/ocr")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&request_body)
            .send()
            .await
            .map_err(|e| OcrError(format!("Failed to send request: {}", e)))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| OcrError(format!("Failed to read response: {}", e)))?;

        if !status.is_success() {
            return Err(OcrError(format!(
                "Failed to perform OCR, status: {}, body: {}",
                status, text
            )));
        }

        let ocr_result: Value =
            serde_json::from_str(&text).map_err(|e| OcrError(format!("Failed to parse response: {}", e)))?;

        self.save_ocr_images(&ocr_result, file, page);

        let ocr_text = self.extract_markdown(&ocr_result, file, page);
        Ok((ocr_text, ocr_result))
    }

    fn provider_id(&self) -> &'static str {
        "mistralocr"
    }
}

impl MistralOcrProvider {
    fn save_ocr_images(&self, ocr_result: &Value, file: &str, page: u32) {
        let Some(pages) = ocr_result.get("pages").and_then(|v| v.as_array()) else {
            return;
        };

        for page_data in pages {
            let Some(images) = page_data.get("images").and_then(|v| v.as_array()) else {
                continue;
            };

            for (img_index, image) in images.iter().enumerate() {
                let Some(image_base64) = image.get("image_base64").and_then(|v| v.as_str()) else {
                    continue;
                };

                let base64_data = image_base64.split(',').nth(1).unwrap_or("");
                let Ok(image_bytes) = base64::engine::general_purpose::STANDARD
                    .decode(base64_data)
                    .map_err(|e| log::error!("Failed to decode base64 image: {}", e))
                else {
                    continue;
                };

                let filename = std::path::Path::new(file)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown");

                let img_output_path = self.config.preview_dir.join(format!(
                    "ocr_image-{}-{}-{}-img-{}.jpeg",
                    self.provider_id(),
                    filename,
                    page,
                    img_index
                ));

                if let Err(e) = std::fs::write(&img_output_path, image_bytes) {
                    log::error!("Failed to write OCR image: {}", e);
                } else {
                    log::info!("Saved OCR image to: {:?}", img_output_path);
                }
            }
        }
    }

    fn extract_markdown(&self, result: &Value, file: &str, page: u32) -> String {
        let Some(pages) = result["pages"].as_array() else {
            return String::new();
        };

        if pages.is_empty() {
            return String::new();
        }

        let re = regex::Regex::new(r"!\[img-(\d+)\.(?:jpeg|jpg|png)\]\(img-\d+\.(?:jpeg|jpg|png)\)")
            .unwrap();

        pages
            .iter()
            .filter_map(|page_data| page_data.get("markdown").and_then(|m| m.as_str()))
            .map(|markdown| {
                re.replace_all(markdown, |caps: &regex::Captures| {
                    let img_index = &caps[1];
                    format!(
                        "![ocr-image]({}/ocr_image/ocr_image-{}-{}-{}-img-{}.jpeg)",
                        self.config.base_url,
                        self.provider_id(),
                        file.replace(".pdf", ""),
                        page,
                        img_index
                    )
                })
                .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}
