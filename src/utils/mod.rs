use base64::{engine::general_purpose, Engine as _};
use std::fs;

pub fn encode_image_to_base64(path: &str) -> Result<String, std::io::Error> {
    let image_data = fs::read(path)?;
    Ok(format!("data:image/png;base64,{}", general_purpose::STANDARD.encode(image_data)))
} 