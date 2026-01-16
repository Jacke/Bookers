use actix_web::{web, Error, HttpResponse};
use log::error;

use crate::models::{OcrResponse, PreviewParams};
use crate::services::{FileService, MistralOcrProvider, OcrProvider};

pub async fn perform_ocr(
    params: web::Path<PreviewParams>,
    file_service: web::Data<FileService>,
) -> Result<HttpResponse, Error> {
    let preview_path = match file_service.generate_preview(&params.file, params.page) {
        Ok(path) => path,
        Err(e) => {
            error!("Failed to generate preview: {}", e);
            return Ok(HttpResponse::InternalServerError()
                .json(OcrResponse { result: format!("Failed to generate preview: {}", e) }));
        }
    };

    let api_key = match std::env::var("MISTRAL_API_KEY") {
        Ok(key) => key,
        Err(_) => {
            error!("MISTRAL_API_KEY not set");
            return Ok(HttpResponse::InternalServerError()
                .json(OcrResponse { result: "MISTRAL_API_KEY not set".to_string() }));
        }
    };

    let provider = MistralOcrProvider::new(api_key);
    match provider
        .extract_text(&preview_path.to_string_lossy(), &params.file, params.page)
        .await
    {
        Ok((ocr_text, ocr_result)) => {
            if let Err(e) =
                file_service.save_ocr_cache(&params.file, params.page, provider.provider_id(), ocr_result)
            {
                error!("Failed to save OCR cache: {}", e);
            }
            Ok(HttpResponse::Ok().json(OcrResponse { result: ocr_text }))
        }
        Err(e) => {
            error!("OCR error: {}", e);
            Ok(HttpResponse::InternalServerError()
                .json(OcrResponse { result: format!("Failed to perform OCR: {}", e) }))
        }
    }
}

pub async fn get_ocr_cache(
    params: web::Path<PreviewParams>,
    file_service: web::Data<FileService>,
) -> Result<HttpResponse, Error> {
    match file_service.get_ocr_cache(&params.file, params.page) {
        Some(data) => Ok(HttpResponse::Ok().content_type("application/json").body(data)),
        None => Ok(HttpResponse::NotFound().body("")),
    }
}
