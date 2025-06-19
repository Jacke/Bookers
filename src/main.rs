mod config;
mod handlers;
mod models;
mod services;
mod utils;
mod constants;

use actix_files::NamedFile;
use actix_web::Result;
use actix_web::web;
use actix_web::{App, HttpServer, HttpResponse, Responder};
use tera::{Tera, Context};
use walkdir::WalkDir;
use actix_files::Files;
use std::process::Command;
use std::fs;
use std::path::{PathBuf, Path};
use serde_json::json;
use log::{info, error, warn, debug};
use std::collections::HashMap;
use serde::Deserialize;
use std::time::Instant;
use actix_web::middleware::Logger;
use dotenv;
use clap::{Parser, Subcommand};
use std::collections::BTreeSet;

// --- OCR Provider Trait and Implementations ---
use async_trait::async_trait;
use std::error::Error;
use crate::services::{OcrProvider, MistralOcrProvider};

async fn index(tmpl: web::Data<Tera>) -> impl Responder {
    let mut context = Context::new();
    let mut files = Vec::new();
    for entry in WalkDir::new(constants::RESOURCES_DIR).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_file() {
            if let Some(ext) = path.extension() {
                if ext == "pdf" || ext == "epub" {
                    if let Some(fname) = path.file_name().and_then(|n| n.to_str()) {
                        files.push(fname.to_string());
                    }
                }
            }
        }
    }
    context.insert("files", &files);
    let rendered = tmpl.render("index.html", &context).unwrap();
    HttpResponse::Ok().content_type("text/html").body(rendered)
}

async fn view_file(query: web::Query<std::collections::HashMap<String, String>>, tmpl: web::Data<Tera>) -> impl Responder {
    let file = query.get("file").cloned().unwrap_or_default();
    let mut context = Context::new();
    context.insert("file", &file);
    let rendered = tmpl.render("pdf_view.html", &context).unwrap();
    HttpResponse::Ok().content_type("text/html").body(rendered)
}

#[derive(Deserialize)]
struct PreviewParams {
    file: String,
    page: u32,
}

use actix_web::{Result as ActixResult, get};

// Handles both preview/{file}/{page} and preview/{image}
async fn get_pdf_preview(path: web::Path<(String, Option<u32>)>) -> actix_web::Result<NamedFile> {
    let (file_or_image, page_opt) = path.into_inner();
    match page_opt {
        Some(page) => {
            // preview/{file}/{page}
            let slug = Path::new(&file_or_image)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(&file_or_image)
                .to_string();
            let preview_path = format!("preview/{}-{}.png", slug, page);
            let alt_path = format!(".preview/ocr-image-{}-{}-img-0.jpeg", slug, page);
            let preview_full_path = Path::new(constants::RESOURCES_DIR).join(&preview_path);
            let alt_full_path = Path::new(constants::RESOURCES_DIR).join(&alt_path);
            if preview_full_path.exists() {
                Ok(NamedFile::open(preview_full_path)?.use_last_modified(true))
            } else if alt_full_path.exists() {
                Ok(NamedFile::open(alt_full_path)?.use_last_modified(true))
            } else {
                Err(actix_web::error::ErrorNotFound("Preview not found"))
            }
        }
        None => {
            // preview/{image}
            let full_path = Path::new(constants::PREVIEW_DIR).join(&file_or_image);
            if full_path.exists() {
                Ok(NamedFile::open(full_path)?.use_last_modified(true))
            } else {
                Err(actix_web::error::ErrorNotFound("Image not found"))
            }
        }
    }
}
/*
#[get("/preview/{image}")]
async fn preview_image(path: web::Path<String>) -> actix_web::Result<NamedFile> {
    let image_name = path.into_inner();
    let full_path = format!(".preview/{}", image_name);
    Ok(NamedFile::open(full_path)?.use_last_modified(true))
}*/

async fn get_pdf_metadata(file: web::Path<String>) -> impl Responder {
    let file_path = PathBuf::from(constants::RESOURCES_DIR).join(&*file);
    println!("Getting metadata for file: {:?}", file_path);
    info!("Getting metadata for file: {:?}", file_path);
    let output = Command::new("pdfinfo")
        .arg(&file_path)
        .output()
        .expect("Failed to execute pdfinfo");
    if !output.status.success() {
        error!("Failed to get metadata: {:?}", output);
        return HttpResponse::InternalServerError().body("Failed to get metadata");
    }
    let metadata_str = String::from_utf8_lossy(&output.stdout);
    let mut metadata = HashMap::new();
    for line in metadata_str.lines() {
        if let Some((key, value)) = line.split_once(":") {
            metadata.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    HttpResponse::Ok()
        .content_type("application/json")
        .body(json!({ "metadata": metadata }).to_string())
}

use base64::{engine::general_purpose, Engine as _};

fn encode_image_to_base64(path: &str) -> Result<String, std::io::Error> {
    let image_data = fs::read(path)?;
    Ok(format!("data:image/png;base64,{}", general_purpose::STANDARD.encode(image_data)))
}

/// Обрабатывает OCR запрос для указанного файла и страницы
/// # Arguments
/// * `params` - Параметры запроса, содержащие имя файла и номер страницы
/// # Returns
/// Результат OCR в формате JSON
async fn perform_ocr(params: web::Path<PreviewParams>) -> impl Responder {
    let file_path = PathBuf::from(constants::RESOURCES_DIR).join(&params.file);
    let preview_dir = PathBuf::from(constants::PREVIEW_DIR);
    let png_path = preview_dir.join(format!("{}_{}.png", params.file.replace("/", "_"), params.page));
    if !png_path.exists() {
        let _ = fs::create_dir_all(&preview_dir);
        let output = Command::new("pdftoppm")
            .arg("-png")
            .arg("-singlefile")
            .arg("-f")
            .arg(params.page.to_string())
            .arg("-l")
            .arg(params.page.to_string())
            .arg(&file_path)
            .arg(png_path.with_extension("").to_string_lossy().to_string())
            .output()
            .expect("Failed to execute pdftoppm");
        if !output.status.success() {
            error!("Failed to generate PNG for OCR: {:?}", output);
            return HttpResponse::InternalServerError().body("Failed to generate PNG for OCR");
        }
    }
    if !png_path.exists() {
        error!("PNG for OCR not found at: {:?}", png_path);
        return HttpResponse::BadRequest().body("PNG for OCR not found");
    }
    // Use provider pattern
    let api_key = match std::env::var("MISTRAL_API_KEY") {
        Ok(key) => key,
        Err(_) => {
            error!("MISTRAL_API_KEY not set");
            return HttpResponse::InternalServerError().body(json!({ "result": "MISTRAL_API_KEY not set" }).to_string());
        }
    };
    let provider = MistralOcrProvider::new(api_key);
    match provider.extract_text(&png_path.to_string_lossy(), &params.file, params.page).await {
        Ok((ocr_text, _ocr_payload)) => {
            HttpResponse::Ok()
                .content_type("application/json")
                .body(json!({ "result": ocr_text }).to_string())
        }
        Err(e) => {
            error!("OCR error: {}", e);
            HttpResponse::InternalServerError().body(json!({ "result": format!("Failed to perform OCR: {}", e) }).to_string())
        }
    }
}

async fn get_ocr_cache(params: web::Path<PreviewParams>) -> impl Responder {
    let ocr_cache_dir = PathBuf::from(constants::OCR_CACHE_DIR);
    let ocr_cache_path = ocr_cache_dir.join(format!("{}_{}.ocr_cache", params.file.replace("/", "_"), params.page));
    if ocr_cache_path.exists() {
        // Read as JSON, return the entire stored JSON object (array of entries)
        match fs::read_to_string(&ocr_cache_path) {
            Ok(data) => {
                // Return as application/json, raw body
                HttpResponse::Ok().content_type("application/json").body(data)
            }
            Err(_) => HttpResponse::InternalServerError().body("Failed to read ocr_cache"),
        }
    } else {
        HttpResponse::NotFound().body("")
    }
}

// Rename duplicate get_ocr_cache to get_ocr_cache_fallback if exists

fn print_ascii_banner(host: &str, port: u16) {
    let banner = r#"
 ____              _              
| __ )  ___   ___ | | _____ _ __  
|  _ \ / _ \ / _ \| |/ / _ \ '__| 
| |_) | (_) | (_) |   <  __/ |    
|____/ \___/ \___/|_|\_\___|_|    
"#;
    println!("{}", banner);
    println!("         Bookers server started at: http://{}:{}\n", host, port);
}

fn load_env() {
    dotenv::dotenv().ok();
}

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Запустить веб-сервер
    Serve,
    /// Вывести markdown OCR результата для файла и страницы
    OcrMarkdown {
        file: String,
        page: String,
    },
    /// Запустить OCR и вывести результат для файла и страницы
    OcrRun {
        file: String,
        page: String,
    },
    /// Показать метаданные PDF (страницы, размеры, автор и т.д.)
    PdfInfo {
        file: String,
    },
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let cli = Cli::parse();
    match &cli.command {
        Some(Commands::Serve) | None => {
            actix_web::rt::System::new().block_on(run_server()).unwrap();
        }
        Some(Commands::OcrMarkdown { file, page }) => {
            let config = config::Config::new();
            let file_service = services::FileService::new(
                config.resources_dir.clone(),
                config.preview_dir.clone(),
                config.ocr_cache_dir.clone(),
            );
            let total_pages = file_service.get_pdf_metadata(file)
                .ok()
                .and_then(|meta| meta.get("Pages").and_then(|v| v.parse::<u32>().ok()))
                .unwrap_or(1);
            let page_range = parse_page_ranges(&page, total_pages);
            for p in page_range {
                let cache_path = std::path::Path::new(constants::OCR_CACHE_DIR)
                    .join(format!("{}_{}.ocr_cache", file.replace("/", "_"), p));
                if !cache_path.exists() {
                    warn!("Нет OCR кэша для файла {} страница {}. Запускаю OCR...", file, p);
                    match run_ocr_for_file_page(file, p) {
                        Ok(result) => {
                            info!("OCR результат: {}", result);
                            println!("--- OCR markdown для страницы {} ---\n{}\n", p, result);
                        }
                        Err(e) => {
                            error!("Ошибка OCR: {}", e);
                        }
                    }
                    continue;
                }
                info!("Нашли OCR кэш для файла {} страница {}", file, p);
                let data = std::fs::read_to_string(&cache_path)
                    .expect("Не удалось прочитать ocr_cache файл");
                let json: serde_json::Value = serde_json::from_str(&data).expect("Некорректный JSON");
                if let Some(entry) = json.as_array().and_then(|arr| arr.get(0)) {
                    if let Some(payload) = entry.get("payload") {
                        if let Some(pages) = payload.get("pages").and_then(|v| v.as_array()) {
                            for (i, pagev) in pages.iter().enumerate() {
                                if let Some(md) = pagev.get("markdown").and_then(|m| m.as_str()) {
                                    println!("--- OCR markdown для страницы {} ---\n{}\n", i+1, md);
                                }
                            }
                        }
                    }
                }
            }
        }
        Some(Commands::OcrRun { file, page }) => {
            let config = config::Config::new();
            let file_service = services::FileService::new(
                config.resources_dir.clone(),
                config.preview_dir.clone(),
                config.ocr_cache_dir.clone(),
            );
            let total_pages = file_service.get_pdf_metadata(file)
                .ok()
                .and_then(|meta| meta.get("Pages").and_then(|v| v.parse::<u32>().ok()))
                .unwrap_or(1);
            let page_range = parse_page_ranges(&page, total_pages);
            for p in page_range {
                match run_ocr_for_file_page(file, p) {
                    Ok(result) => {
                        info!("OCR результат: {}", result);
                        println!("--- OCR результат для страницы {} ---\n{}\n", p, result);
                    }
                    Err(e) => {
                        error!("Ошибка OCR: {}", e);
                    }
                }
            }
        }
        Some(Commands::PdfInfo { file }) => {
            let config = config::Config::new();
            let file_service = services::FileService::new(
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
                    eprintln!("Ошибка получения метаданных: {}", e);
                }
            }
        }
    }
}

async fn run_server() -> std::io::Result<()> {
    load_env();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .init();
    let config = config::Config::new();
    let host = config.host.clone();
    let port = config.port;
    print_ascii_banner(&host, port);
    info!("🚀 Server running at http://{}:{}/", host, port);
    let startup_time = Instant::now();
    let tera = Tera::new("templates/**/*")
        .expect("Failed to initialize Tera templates");
    let file_service = services::FileService::new(
        config.resources_dir.clone(),
        config.preview_dir.clone(),
        config.ocr_cache_dir.clone(),
    );
    HttpServer::new(move || {
        App::new()
            .wrap(Logger::default())
            .app_data(web::Data::new(tera.clone()))
            .app_data(web::Data::new(config.clone()))
            .app_data(web::Data::new(file_service.clone()))
            .route("/", web::get().to(handlers::index))
            .route("/view", web::get().to(handlers::view_file))
            .route("/preview/{filename}/{page}", web::get().to(handlers::get_pdf_preview))
            .route("/preview_image/{filename}/{page}", web::get().to(handlers::get_preview_image))
            .route("/metadata/{file}", web::get().to(handlers::get_pdf_metadata))
            .route("/ocr/{file}/{page}", web::post().to(handlers::perform_ocr))
            .route("/ocr_cache/{file}/{page}", web::get().to(handlers::get_ocr_cache))
            .route("/ocr_image/{filename:.*}", web::get().to(handlers::get_ocr_image))
            .route("/generate_all_previews/{file:.*}", web::post().to(handlers::generate_all_previews))
            .route("/generation_status/{file:.*}", web::get().to(handlers::get_generation_status))
            .route("/healthz", web::get().to(|| async { "OK" }))
            .service(Files::new("/static", "static").show_files_listing())
    })
    .bind((host, port))?
    .run()
    .await?;
    info!("🛑 Server stopped. Uptime: {:?}", startup_time.elapsed());
    Ok(())
}

// Handler to serve files from .preview/ directory via /preview/{filename:.*}
async fn preview_file(path: web::Path<String>) -> actix_web::Result<NamedFile> {
    let filename = path.into_inner();
    let full_path = format!(".preview/{}", filename);
    match actix_files::NamedFile::open_async(full_path).await {
        Ok(file) => Ok(file.use_last_modified(true)),
        Err(_) => Err(actix_web::error::ErrorNotFound("File not found")),
    }
}
// Handler to serve fallback OCR cache (renamed from duplicate get_ocr_cache)
async fn get_ocr_cache_fallback(params: web::Path<PreviewParams>) -> impl Responder {
    let ocr_cache_dir = PathBuf::from(constants::OCR_CACHE_DIR);
    let ocr_cache_path = ocr_cache_dir.join(format!("{}_{}.ocr_cache", params.file.replace("/", "_"), params.page));
    if ocr_cache_path.exists() {
        // Read as JSON, return the entire stored JSON object (array of entries)
        match fs::read_to_string(&ocr_cache_path) {
            Ok(data) => {
                // Return as application/json, raw body
                HttpResponse::Ok().content_type("application/json").body(data)
            }
            Err(_) => HttpResponse::InternalServerError().body("Failed to read ocr_cache"),
        }
    } else {
        HttpResponse::NotFound().body("")
    }
}

async fn preview_image(path: web::Path<(String, u32)>) -> actix_web::Result<NamedFile> {
    let (filename, page) = path.into_inner();
    let image_path = format!("./resources/.preview/{}_{}.jpg", filename, page);
    NamedFile::open(image_path).map_err(|e| actix_web::error::ErrorInternalServerError(e))
}

#[derive(Debug, Deserialize)]
pub struct Config {
    pub preview_dir: String,
    pub ocr_cache_dir: String,
    pub server_port: u16,
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_ocr_provider() {
        // Тесты для OCR провайдера
    }
}

fn run_ocr_for_file_page(file: &str, page: u32) -> Result<String, String> {
    use crate::services::FileService;
    use crate::services::MistralOcrProvider;
    use std::env;
    use std::path::PathBuf;
    use log::{error, info};

    let config = config::Config::new();
    let file_service = FileService::new(
        config.resources_dir.clone(),
        config.preview_dir.clone(),
        config.ocr_cache_dir.clone(),
    );

    // Генерируем превью (png)
    let preview_path = match file_service.generate_preview(file, page) {
        Ok(path) => path,
        Err(e) => {
            error!("Failed to generate preview: {}", e);
            return Err(format!("Failed to generate preview: {}", e));
        }
    };

    // Получаем API-ключ
    let api_key = match env::var("MISTRAL_API_KEY") {
        Ok(key) => key,
        Err(_) => {
            error!("MISTRAL_API_KEY not set");
            return Err("MISTRAL_API_KEY not set".to_string());
        }
    };

    // Запускаем OCR (блокирующий вызов)
    let provider = MistralOcrProvider::new(api_key);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let ocr_result = rt.block_on(provider.extract_text(
        &preview_path.to_string_lossy(),
        file,
        page,
    ));
    match ocr_result {
        Ok((ocr_text, ocr_payload)) => {
            if let Err(e) = file_service.save_ocr_cache(
                file,
                page,
                provider.provider_id(),
                ocr_payload
            ) {
                error!("Failed to save OCR cache: {}", e);
            }
            Ok(ocr_text)
        }
        Err(e) => {
            error!("OCR error: {}", e);
            Err(format!("Failed to perform OCR: {}", e))
        }
    }
}

fn parse_page_ranges(range_str: &str, total_pages: u32) -> BTreeSet<u32> {
    let mut pages = BTreeSet::new();
    for part in range_str.split(',') {
        let part = part.trim();
        if part.is_empty() { continue; }
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
        } else {
            if let Ok(p) = part.parse::<u32>() {
                pages.insert(p);
            }
        }
    }
    pages
}