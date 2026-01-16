use clap::{Parser, Subcommand};
use log::{error, info, warn};
use std::collections::BTreeSet;

use crate::config::Config;
use crate::services::{FileService, MistralOcrProvider, OcrProvider};

#[derive(Parser)]
#[command(name = "booker")]
#[command(author, version, about = "PDF/EPUB reader with OCR support", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Start the web server
    Serve,

    /// Output OCR markdown for a file and page(s)
    OcrMarkdown {
        /// PDF filename
        file: String,
        /// Page number or range (e.g., "1", "1-5", "1,3,5", "1-e" for all)
        page: String,
    },

    /// Run OCR and output result for a file and page(s)
    OcrRun {
        /// PDF filename
        file: String,
        /// Page number or range (e.g., "1", "1-5", "1,3,5", "1-e" for all)
        page: String,
    },

    /// Show PDF metadata (pages, dimensions, author, etc.)
    PdfInfo {
        /// PDF filename
        file: String,
    },
}

pub fn handle_ocr_markdown(file: &str, page: &str) {
    let config = Config::new();
    let file_service = FileService::new(
        config.resources_dir.clone(),
        config.preview_dir.clone(),
        config.ocr_cache_dir.clone(),
    );

    let total_pages = file_service
        .get_pdf_metadata(file)
        .ok()
        .and_then(|meta| meta.get("Pages").and_then(|v| v.parse::<u32>().ok()))
        .unwrap_or(1);

    let page_range = parse_page_ranges(page, total_pages);

    for p in page_range {
        let cache_path = config.ocr_cache_dir.join(format!(
            "{}_{}.ocr_cache",
            file.replace('/', "_"),
            p
        ));

        if !cache_path.exists() {
            warn!("No OCR cache for file {} page {}. Running OCR...", file, p);
            match run_ocr_for_file_page(file, p, &config) {
                Ok(result) => {
                    info!("OCR result: {}", result);
                    println!("--- OCR markdown for page {} ---\n{}\n", p, result);
                }
                Err(e) => {
                    error!("OCR error: {}", e);
                }
            }
            continue;
        }

        info!("Found OCR cache for file {} page {}", file, p);
        let data = std::fs::read_to_string(&cache_path).expect("Failed to read ocr_cache file");
        let json: serde_json::Value = serde_json::from_str(&data).expect("Invalid JSON");

        if let Some(entry) = json.as_array().and_then(|arr| arr.first()) {
            if let Some(payload) = entry.get("payload") {
                if let Some(pages) = payload.get("pages").and_then(|v| v.as_array()) {
                    for (i, page_value) in pages.iter().enumerate() {
                        if let Some(md) = page_value.get("markdown").and_then(|m| m.as_str()) {
                            println!("--- OCR markdown for page {} ---\n{}\n", i + 1, md);
                        }
                    }
                }
            }
        }
    }
}

pub fn handle_ocr_run(file: &str, page: &str) {
    let config = Config::new();
    let file_service = FileService::new(
        config.resources_dir.clone(),
        config.preview_dir.clone(),
        config.ocr_cache_dir.clone(),
    );

    let total_pages = file_service
        .get_pdf_metadata(file)
        .ok()
        .and_then(|meta| meta.get("Pages").and_then(|v| v.parse::<u32>().ok()))
        .unwrap_or(1);

    let page_range = parse_page_ranges(page, total_pages);

    for p in page_range {
        match run_ocr_for_file_page(file, p, &config) {
            Ok(result) => {
                info!("OCR result: {}", result);
                println!("--- OCR result for page {} ---\n{}\n", p, result);
            }
            Err(e) => {
                error!("OCR error: {}", e);
            }
        }
    }
}

pub fn handle_pdf_info(file: &str) {
    let config = Config::new();
    let file_service = FileService::new(
        config.resources_dir.clone(),
        config.preview_dir.clone(),
        config.ocr_cache_dir.clone(),
    );

    match file_service.get_pdf_metadata(file) {
        Ok(metadata) => {
            println!("PDF metadata for '{}':", file);
            for (k, v) in &metadata {
                println!("{:20}: {}", k, v);
            }
        }
        Err(e) => {
            eprintln!("Error getting metadata: {}", e);
        }
    }
}

fn run_ocr_for_file_page(file: &str, page: u32, config: &Config) -> Result<String, String> {
    let file_service = FileService::new(
        config.resources_dir.clone(),
        config.preview_dir.clone(),
        config.ocr_cache_dir.clone(),
    );

    let preview_path = file_service
        .generate_preview(file, page)
        .map_err(|e| format!("Failed to generate preview: {}", e))?;

    let api_key = std::env::var("MISTRAL_API_KEY")
        .map_err(|_| "MISTRAL_API_KEY not set".to_string())?;

    let provider = MistralOcrProvider::new(api_key);
    let rt = tokio::runtime::Runtime::new().unwrap();

    let ocr_result = rt.block_on(provider.extract_text(
        &preview_path.to_string_lossy(),
        file,
        page,
    ));

    match ocr_result {
        Ok((ocr_text, ocr_payload)) => {
            if let Err(e) = file_service.save_ocr_cache(file, page, provider.provider_id(), ocr_payload) {
                error!("Failed to save OCR cache: {}", e);
            }
            Ok(ocr_text)
        }
        Err(e) => Err(format!("Failed to perform OCR: {}", e)),
    }
}

fn parse_page_ranges(range_str: &str, total_pages: u32) -> BTreeSet<u32> {
    let mut pages = BTreeSet::new();

    for part in range_str.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }

        if let Some((start, end)) = part.split_once('-') {
            let start = start.trim().parse::<u32>().unwrap_or(1);
            let end = if end.trim() == "e" {
                total_pages
            } else {
                end.trim().parse::<u32>().unwrap_or(start)
            };
            for p in start..=end {
                pages.insert(p);
            }
        } else if let Ok(p) = part.parse::<u32>() {
            pages.insert(p);
        }
    }

    pages
}
