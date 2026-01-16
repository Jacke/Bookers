use actix_web::{web, Error, HttpResponse};
use tera::{Context, Tera};
use walkdir::WalkDir;

use crate::config::Config;

pub async fn index(tmpl: web::Data<Tera>, config: web::Data<Config>) -> Result<HttpResponse, Error> {
    let mut context = Context::new();
    let mut files = Vec::new();

    for entry in WalkDir::new(&config.resources_dir)
        .into_iter()
        .filter_map(|e| e.ok())
    {
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
    let rendered = tmpl.render("index.html", &context).map_err(|e| {
        log::error!("Template error: {}", e);
        actix_web::error::ErrorInternalServerError(e)
    })?;

    Ok(HttpResponse::Ok().content_type("text/html").body(rendered))
}

pub async fn view_file(
    query: web::Query<std::collections::HashMap<String, String>>,
    tmpl: web::Data<Tera>,
) -> Result<HttpResponse, Error> {
    let file = query.get("file").cloned().unwrap_or_default();
    let mut context = Context::new();
    context.insert("file", &file);

    let rendered = tmpl.render("pdf_view.html", &context).map_err(|e| {
        log::error!("Template error: {}", e);
        actix_web::error::ErrorInternalServerError(e)
    })?;

    Ok(HttpResponse::Ok().content_type("text/html").body(rendered))
}
