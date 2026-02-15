use actix_files::Files;
use actix_web::middleware::Logger;
use actix_web::{web, App, HttpServer};
use log::info;
use std::sync::Arc;
use std::time::Instant;
use tera::Tera;

use crate::config::Config;
use crate::handlers;
use crate::services::{FileService, database::Database, background::JobManager};

pub async fn run() -> std::io::Result<()> {
    let config = Config::new();
    let host = config.host.clone();
    let port = config.port;

    print_banner(&host, port);
    info!("Server running at http://{}:{}/", host, port);

    let startup_time = Instant::now();
    let mut tera = Tera::new("templates/**/*").expect("Failed to initialize Tera templates");
    
    // Register markdown filter
    tera.register_filter("markdown", |value: &tera::Value, _args: &std::collections::HashMap<String, tera::Value>| {
        let text = value.as_str().unwrap_or("");
        // Simple markdown to HTML conversion
        let html = text
            .replace("**", "<strong>")
            .replace("*", "<em>")
            .replace("`", "<code>")
            .replace("\n\n", "</p><p>")
            .replace("\n", "<br>");
        Ok(tera::Value::String(format!("<p>{}</p>", html)))
    });
    
    // Register truncate filter
    tera.register_filter("truncate", |value: &tera::Value, args: &std::collections::HashMap<String, tera::Value>| {
        let text = value.as_str().unwrap_or("");
        let length = args.get("length").and_then(|v| v.as_i64()).unwrap_or(100) as usize;
        if text.len() > length {
            Ok(tera::Value::String(format!("{}...", &text[..length])))
        } else {
            Ok(tera::Value::String(text.to_string()))
        }
    });

    let file_service = FileService::new(
        config.resources_dir.clone(),
        config.preview_dir.clone(),
        config.ocr_cache_dir.clone(),
    );

    // Initialize database
    std::fs::create_dir_all("data").expect("Failed to create data directory");
    // Use file-based database for persistence, create file if not exists
    let db_path = std::env::current_dir().unwrap().join("data/textbooks.db");
    if !db_path.exists() {
        std::fs::File::create(&db_path).expect("Failed to create database file");
    }
    let db_url = format!("sqlite:{}", db_path.to_str().unwrap());
    let database = Database::new(&db_url)
        .await
        .expect("Failed to initialize database");

    // Initialize job manager for background tasks
    let job_manager = Arc::new(JobManager::new());
    
    // Spawn cleanup task for old jobs
    let cleanup_jobs = job_manager.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(3600)); // Every hour
        loop {
            interval.tick().await;
            cleanup_jobs.cleanup_old_jobs().await;
        }
    });

    HttpServer::new(move || {
        App::new()
            .wrap(Logger::default())
            .app_data(web::Data::new(tera.clone()))
            .app_data(web::Data::new(config.clone()))
            .app_data(web::Data::new(file_service.clone()))
            .app_data(web::Data::new(database.clone()))
            .app_data(web::Data::new(job_manager.clone()))
            .configure(configure_routes)
    })
    .bind((host, port))?
    .run()
    .await?;

    info!("Server stopped. Uptime: {:?}", startup_time.elapsed());
    Ok(())
}

fn configure_routes(cfg: &mut web::ServiceConfig) {
    // Static and main pages
    cfg.route("/", web::get().to(handlers::index))
        .route("/view", web::get().to(handlers::view_file))
        .service(Files::new("/static", "static").show_files_listing());

    // Preview and OCR routes (existing)
    cfg.route(
            "/preview/{filename}/{page}",
            web::get().to(handlers::get_pdf_preview),
        )
        .route(
            "/preview_image/{filename}/{page}",
            web::get().to(handlers::get_preview_image),
        )
        .route("/metadata/{file}", web::get().to(handlers::get_pdf_metadata))
        .route("/ocr/{file}/{page}", web::post().to(handlers::perform_ocr))
        .route("/api/ocr_page/{filename}/{page}", web::post().to(handlers::ocr_pdf_page))
        .route("/api/page_ocr/{book_id}/{page}", web::get().to(handlers::get_page_ocr))
        .route("/api/parse_problems", web::post().to(handlers::parse_problems_from_text))
        .route("/api/parse_full_page", web::post().to(handlers::parse_full_page))
        .route("/api/problems/bulk_create", web::post().to(handlers::create_problems_from_ocr))
        .route("/api/pages/{page_id}/problems", web::get().to(handlers::get_problems_by_page))
        .route(
            "/ocr_cache/{file}/{page}",
            web::get().to(handlers::get_ocr_cache),
        )
        .route(
            "/ocr_image/{filename:.*}",
            web::get().to(handlers::get_ocr_image),
        )
        .route(
            "/generate_all_previews/{file:.*}",
            web::post().to(handlers::generate_all_previews),
        )
        .route(
            "/generation_status/{file:.*}",
            web::get().to(handlers::get_generation_status),
        );

    // Textbook HTML views
    cfg.route(
            "/textbook/book/{book_id}/pages",
            web::get().to(handlers::view_book_pages),
        )
        .route(
            "/textbook/book/{book_id}/page/{page_number}",
            web::get().to(handlers::view_page),
        )
        .route(
            "/textbook/chapter/{chapter_id}",
            web::get().to(handlers::view_chapter),
        )
        .route(
            "/textbook/problem/{problem_id}",
            web::get().to(handlers::view_problem),
        );

    // Problem API routes
    cfg.route(
            "/api/chapters/{chapter_id}/problems",
            web::get().to(handlers::get_chapter_problems),
        )
        .route(
            "/api/chapters/{chapter_id}/theory",
            web::get().to(handlers::get_chapter_theory),
        )
        .route(
            "/api/problems/{problem_id}",
            web::get().to(handlers::get_problem),
        )
        .route(
            "/api/problems/{problem_id}",
            web::put().to(handlers::update_problem),
        )
        .route(
            "/api/problems/{problem_id}/solve",
            web::post().to(handlers::solve_problem),
        )
        .route(
            "/api/problems/{problem_id}/solution",
            web::put().to(handlers::save_solution),
        )
        .route(
            "/api/problems/{problem_id}/solutions/{solution_id}/rate",
            web::post().to(handlers::rate_solution),
        )
        .route(
            "/api/problems/{problem_id}/hint",
            web::post().to(handlers::hint_problem),
        )
        .route(
            "/api/import",
            web::post().to(handlers::import_textbook),
        )
        .route(
            "/api/bookmarks/{problem_id}",
            web::post().to(handlers::add_bookmark),
        )
        .route(
            "/api/bookmarks/{problem_id}",
            web::delete().to(handlers::remove_bookmark),
        )
        .route(
            "/api/bookmarks",
            web::get().to(handlers::list_bookmarks),
        )
        .route(
            "/api/history/view/{problem_id}",
            web::post().to(handlers::record_view),
        )
        .route(
            "/api/history",
            web::get().to(handlers::get_view_history),
        )
        .route(
            "/api/history",
            web::delete().to(handlers::clear_view_history),
        );
    
    // Search route
    cfg.route("/api/search", web::get().to(handlers::search_problems));
    
    // Batch processing routes
    cfg.route("/api/batch/ocr", web::post().to(handlers::start_batch_ocr))
        .route("/api/batch/solve", web::post().to(handlers::start_batch_solve))
        .route("/api/jobs", web::get().to(handlers::list_jobs))
        .route("/api/jobs/{job_id}", web::get().to(handlers::get_job_status))
        .route("/api/jobs/{job_id}/cancel", web::post().to(handlers::cancel_job));
    
    // Export routes
    cfg.route("/api/export/book", web::post().to(handlers::export_book))
        .route("/api/export/chapter/{chapter_id}", web::get().to(handlers::export_chapter));
    
    // Validation routes
    cfg.route("/api/validate/chapter", web::post().to(handlers::validate_chapter));
    
    // Formula search
    cfg.route("/api/search/formula", web::post().to(handlers::search_by_formula));
    
    // WebSocket for job progress
    cfg.route("/ws/jobs", web::get().to(handlers::job_websocket));
    
    // Smart features - TOC detection
    cfg.route("/api/smart/detect_toc", web::post().to(handlers::detect_toc))
        .route("/api/smart/import_book", web::post().to(handlers::smart_import_book));
    
    // Knowledge Graph
    cfg.route("/api/graph/build", web::post().to(handlers::build_knowledge_graph));
    
    // Auto-tagging
    cfg.route("/api/smart/auto_tag", web::post().to(handlers::auto_tag_problems));
    
    // Similarity & Recommendations
    cfg.route("/api/smart/similar", web::post().to(handlers::find_similar_problems))
        .route("/api/smart/recommend", web::post().to(handlers::recommend_problems))
        .route("/api/smart/duplicates", web::post().to(handlers::find_duplicates));
        
    // Health check
    cfg.route("/healthz", web::get().to(|| async { "OK" }));
}

fn print_banner(host: &str, port: u16) {
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
