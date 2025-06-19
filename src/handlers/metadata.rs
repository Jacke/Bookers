use actix_web::{web, HttpResponse, Error};
use crate::models::MetadataResponse;
use crate::services::FileService;
use log::error;

pub async fn get_pdf_metadata(
    file: web::Path<String>,
    file_service: web::Data<FileService>,
) -> Result<HttpResponse, Error> {
    match file_service.get_pdf_metadata(&file) {
        Ok(metadata) => Ok(HttpResponse::Ok()
            .json(MetadataResponse { metadata })),
        Err(e) => {
            error!("Failed to get metadata: {}", e);
            Ok(HttpResponse::InternalServerError().body("Failed to get metadata"))
        }
    }
} 