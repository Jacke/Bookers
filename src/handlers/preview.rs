use actix_files::NamedFile;
use actix_web::{web, Error, HttpResponse};
use log::{error, info};
use std::collections::HashMap;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::thread;
use tokio::sync::Mutex;

use crate::models::PreviewImageParams;
use crate::services::FileService;

#[derive(Clone)]
struct GenerationProgress {
    total_pages: u32,
    processed_pages: Arc<AtomicU32>,
    is_complete: Arc<AtomicU32>,
}

lazy_static::lazy_static! {
    static ref GENERATION_PROGRESS: Mutex<HashMap<String, GenerationProgress>> = Mutex::new(HashMap::new());
}

pub async fn get_preview_image(
    path: web::Path<PreviewImageParams>,
    file_service: web::Data<FileService>,
) -> Result<HttpResponse, Error> {
    let preview_path = file_service
        .get_preview_dir()
        .join(format!("{}_{}.jpg", path.filename, path.page));

    if !preview_path.exists() {
        return Ok(HttpResponse::NotFound().body("Image not found"));
    }

    match std::fs::read(&preview_path) {
        Ok(data) => Ok(HttpResponse::Ok().content_type("image/jpeg").body(data)),
        Err(e) => {
            error!("Failed to read image file: {}", e);
            Ok(HttpResponse::InternalServerError().body("Failed to read image file"))
        }
    }
}

pub async fn get_pdf_preview(
    path: web::Path<(String, Option<u32>)>,
    file_service: web::Data<FileService>,
) -> actix_web::Result<NamedFile> {
    let (file_or_image, page_opt) = path.into_inner();

    match page_opt {
        Some(page) => {
            let preview_path = file_service.generate_preview(&file_or_image, page).map_err(|e| {
                error!("Failed to generate preview: {}", e);
                actix_web::error::ErrorInternalServerError(e)
            })?;

            Ok(NamedFile::open(preview_path)?.use_last_modified(true))
        }
        None => {
            let full_path = file_service.get_preview_dir().join(&file_or_image);
            if full_path.exists() {
                Ok(NamedFile::open(full_path)?.use_last_modified(true))
            } else {
                Err(actix_web::error::ErrorNotFound("Image not found"))
            }
        }
    }
}

pub async fn get_ocr_image(
    path: web::Path<String>,
    file_service: web::Data<FileService>,
) -> Result<HttpResponse, Error> {
    let filename = path.into_inner();
    let full_path = file_service.get_preview_dir().join(&filename);

    log::info!("Looking for OCR image at: {:?}", full_path);

    if !full_path.exists() {
        log::error!("OCR image not found at: {:?}", full_path);
        return Ok(HttpResponse::NotFound().body("OCR image not found"));
    }

    match std::fs::read(&full_path) {
        Ok(data) => Ok(HttpResponse::Ok().content_type("image/jpeg").body(data)),
        Err(e) => {
            log::error!("Failed to read OCR image file: {}", e);
            Ok(HttpResponse::InternalServerError().body("Failed to read OCR image file"))
        }
    }
}

pub async fn get_generation_status(path: web::Path<String>) -> Result<HttpResponse, Error> {
    let file = path.into_inner();
    let progress = GENERATION_PROGRESS.lock().await;

    if let Some(gen_progress) = progress.get(&file) {
        let processed = gen_progress.processed_pages.load(Ordering::Relaxed);
        let total = gen_progress.total_pages;
        let is_complete = gen_progress.is_complete.load(Ordering::Relaxed) == 1;

        Ok(HttpResponse::Ok().json(serde_json::json!({
            "processed_pages": processed,
            "total_pages": total,
            "is_complete": is_complete
        })))
    } else {
        Ok(HttpResponse::NotFound().json(serde_json::json!({
            "error": "No generation in progress for this file"
        })))
    }
}

pub async fn generate_all_previews(
    file_service: web::Data<FileService>,
    path: web::Path<String>,
) -> Result<HttpResponse, Error> {
    let file = path.into_inner();
    let file_path = file_service.get_resources_dir().join(&file);

    if !file_path.exists() {
        return Ok(HttpResponse::NotFound().json(serde_json::json!({
            "error": "File not found"
        })));
    }

    let output = Command::new("pdfinfo").arg(&file_path).output().map_err(|e| {
        error!("Failed to execute pdfinfo: {}", e);
        actix_web::error::ErrorInternalServerError(e)
    })?;

    if !output.status.success() {
        error!("Failed to get PDF info for {:?}: {:?}", file_path, output);
        return Ok(HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Failed to get PDF info"
        })));
    }

    let metadata = String::from_utf8_lossy(&output.stdout);
    let total_pages = metadata
        .lines()
        .find(|line| line.starts_with("Pages:"))
        .and_then(|line| line.split_once(':'))
        .and_then(|(_, num)| num.trim().parse::<u32>().ok())
        .unwrap_or(0);

    if total_pages == 0 {
        return Ok(HttpResponse::BadRequest().json(serde_json::json!({
            "error": "No pages found in PDF"
        })));
    }

    let progress = GenerationProgress {
        total_pages,
        processed_pages: Arc::new(AtomicU32::new(0)),
        is_complete: Arc::new(AtomicU32::new(0)),
    };

    {
        let mut progress_map = GENERATION_PROGRESS.lock().await;
        progress_map.insert(file.clone(), progress.clone());
    }

    let file_service = Arc::new(file_service);
    let file_clone = file.clone();
    let progress_clone = progress.clone();

    tokio::spawn(async move {
        let thread_id = thread::current().id();
        info!(
            "[Thread {:?}] Starting preview generation for {} ({} pages)",
            thread_id, file_clone, total_pages
        );

        for page in 1..=total_pages {
            info!(
                "[Thread {:?}] Generating preview for {} - page {}/{}",
                thread_id, file_clone, page, total_pages
            );
            match file_service.generate_preview(&file_clone, page) {
                Ok(_) => {
                    info!(
                        "[Thread {:?}] Successfully generated preview for {} - page {}/{}",
                        thread_id, file_clone, page, total_pages
                    );
                    progress_clone.processed_pages.fetch_add(1, Ordering::Relaxed);
                }
                Err(e) => error!(
                    "[Thread {:?}] Failed to generate preview for {} page {}/{}: {}",
                    thread_id, file_clone, page, total_pages, e
                ),
            }
        }
        progress_clone.is_complete.store(1, Ordering::Relaxed);
        info!(
            "[Thread {:?}] Finished generating all previews for {} ({} pages total)",
            thread_id, file_clone, total_pages
        );
    });

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "message": "Preview generation started",
        "total_pages": total_pages
    })))
}
