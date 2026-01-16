use actix_files::Files;
use actix_web::middleware::Logger;
use actix_web::{web, App, HttpServer};
use log::info;
use std::time::Instant;
use tera::Tera;

use crate::config::Config;
use crate::handlers;
use crate::services::FileService;

pub async fn run() -> std::io::Result<()> {
    let config = Config::new();
    let host = config.host.clone();
    let port = config.port;

    print_banner(&host, port);
    info!("Server running at http://{}:{}/", host, port);

    let startup_time = Instant::now();
    let tera = Tera::new("templates/**/*").expect("Failed to initialize Tera templates");

    let file_service = FileService::new(
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
            .configure(configure_routes)
    })
    .bind((host, port))?
    .run()
    .await?;

    info!("Server stopped. Uptime: {:?}", startup_time.elapsed());
    Ok(())
}

fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.route("/", web::get().to(handlers::index))
        .route("/view", web::get().to(handlers::view_file))
        .route(
            "/preview/{filename}/{page}",
            web::get().to(handlers::get_pdf_preview),
        )
        .route(
            "/preview_image/{filename}/{page}",
            web::get().to(handlers::get_preview_image),
        )
        .route("/metadata/{file}", web::get().to(handlers::get_pdf_metadata))
        .route("/ocr/{file}/{page}", web::post().to(handlers::perform_ocr))
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
        )
        .route("/healthz", web::get().to(|| async { "OK" }))
        .service(Files::new("/static", "static").show_files_listing());
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
