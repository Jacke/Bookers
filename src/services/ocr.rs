use crate::models::OcrError;
use async_trait::async_trait;
use serde_json::Value;
use base64::{engine::general_purpose::STANDARD, Engine};

#[async_trait]
pub trait OcrProvider: Send + Sync {
    async fn extract_text(&self, image_path: &str, file: &str, page: u32) -> Result<(String, Value), OcrError>;
    fn provider_id(&self) -> &'static str;
}

pub struct MistralOcrProvider {
    api_key: String,
}

impl MistralOcrProvider {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }
}

#[async_trait]
impl OcrProvider for MistralOcrProvider {
    async fn extract_text(&self, image_path: &str, file: &str, page: u32) -> Result<(String, Value), OcrError> {
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
        let text = resp.text().await
            .map_err(|e| OcrError(format!("Failed to read response: {}", e)))?;

        if !status.is_success() {
            return Err(OcrError(format!("Failed to perform OCR, status: {}, body: {}", status, text)));
        }

        let ocr_result: Value = serde_json::from_str(&text)
            .map_err(|e| OcrError(format!("Failed to parse response: {}", e)))?;

        // Save OCR images
        if let Some(pages) = ocr_result.get("pages").and_then(|v| v.as_array()) {
            for page_data in pages {
                if let Some(images) = page_data.get("images").and_then(|v| v.as_array()) {
                    for (img_index, image) in images.iter().enumerate() {
                        if let Some(image_base64) = image.get("image_base64").and_then(|v| v.as_str()) {
                            let base64_data = image_base64.split(',').nth(1).unwrap_or("");
                            if let Ok(image_bytes) = base64::engine::general_purpose::STANDARD.decode(base64_data) {
                                let filename = std::path::Path::new(file)
                                    .file_stem()
                                    .and_then(|s| s.to_str())
                                    .unwrap_or("unknown");
                                let img_output_path = format!(
                                    "./resources/.preview/ocr_image-{}-{}-{}-img-{}.jpeg",
                                    self.provider_id(),
                                    filename,
                                    page,
                                    img_index
                                );
                                if let Err(e) = std::fs::write(&img_output_path, image_bytes) {
                                    log::error!("Failed to write OCR image: {}", e);
                                } else {
                                    log::info!("Saved OCR image to: {}", img_output_path);
                                }
                            }
                        }
                    }
                }
            }
        }

        let ocr_text = extract_text_from_result(&ocr_result, file, page, self.provider_id());
        Ok((ocr_text, ocr_result))
    }

    fn provider_id(&self) -> &'static str {
        "mistralocr"
    }
}

fn extract_text_from_result(result: &Value, file: &str, page: u32, provider_id: &str) -> String {
    if let Some(pages) = result["pages"].as_array() {
        if pages.is_empty() {
            return String::new();
        }

        pages.iter()
            .filter_map(|page| page.get("markdown").and_then(|m| m.as_str()))
            .map(|markdown| {
                let mut replaced = markdown.to_string();
                let re = regex::Regex::new(r"!\[img-(\d+)\.(?:jpeg|jpg|png)\]\(img-\d+\.(?:jpeg|jpg|png)\)").unwrap();
                replaced = re.replace_all(&replaced, |caps: &regex::Captures| {
                    let img_index = &caps[1];
                    format!(
                        "![ocr-image](http://127.0.0.1:8080/ocr_image/ocr_image-{}-{}-{}-img-{}.jpeg)",
                        provider_id,
                        file.replace(".pdf", ""),
                        page,
                        img_index
                    )
                }).to_string();
                replaced
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    } else {
        String::new()
    }
} 