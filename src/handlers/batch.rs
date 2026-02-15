use actix_web::{web, Error, HttpResponse};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::config::Config;
use crate::services::background::{JobManager, JobStatus};
use crate::services::batch_processor::BatchProcessor;
use crate::services::database::Database;

// === Batch OCR ===

#[derive(Debug, Deserialize)]
pub struct BatchOcrRequest {
    pub book_id: String,
    pub start_page: u32,
    pub end_page: u32,
    pub chapter_id: String,
    /// If true, skip pages that already have OCR cached
    pub incremental: Option<bool>,
    /// If true, force re-OCR even if cached
    pub force: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct BatchOcrResponse {
    pub job_id: String,
    pub status: String,
    pub message: String,
    pub total_pages: u32,
}

pub async fn start_batch_ocr(
    body: web::Json<BatchOcrRequest>,
    job_manager: web::Data<Arc<JobManager>>,
    db: web::Data<Database>,
    config: web::Data<Config>,
) -> Result<HttpResponse, Error> {
    // Validate page range
    if body.start_page > body.end_page {
        return Ok(HttpResponse::BadRequest().json(serde_json::json!({
            "error": "Invalid page range: start_page must be <= end_page"
        })));
    }
    
    if body.end_page - body.start_page > 100 {
        return Ok(HttpResponse::BadRequest().json(serde_json::json!({
            "error": "Page range too large (max 100 pages per batch)"
        })));
    }
    
    let processor = BatchProcessor::new(
        job_manager.get_ref().clone(),
        Arc::new(db.get_ref().clone()),
        Arc::new(config.get_ref().clone()),
    );
    
    let incremental = body.incremental.unwrap_or(false);
    let force = body.force.unwrap_or(false);
    
    match processor.start_batch_ocr(&body.book_id, body.start_page, body.end_page, &body.chapter_id, incremental, force).await {
        Ok(job_id) => {
            Ok(HttpResponse::Accepted().json(BatchOcrResponse {
                job_id,
                status: "pending".to_string(),
                message: format!("Batch OCR started for pages {}-{}", body.start_page, body.end_page),
                total_pages: body.end_page - body.start_page + 1,
            }))
        }
        Err(e) => {
            log::error!("Failed to start batch OCR: {}", e);
            Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Failed to start batch OCR: {}", e)
            })))
        }
    }
}

// === Batch Solve ===

#[derive(Debug, Deserialize)]
pub struct BatchSolveRequest {
    pub problem_ids: Vec<String>,
    pub provider: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct BatchSolveResponse {
    pub job_id: String,
    pub status: String,
    pub message: String,
    pub total_problems: usize,
}

pub async fn start_batch_solve(
    body: web::Json<BatchSolveRequest>,
    job_manager: web::Data<Arc<JobManager>>,
    db: web::Data<Database>,
    config: web::Data<Config>,
) -> Result<HttpResponse, Error> {
    if body.problem_ids.is_empty() {
        return Ok(HttpResponse::BadRequest().json(serde_json::json!({
            "error": "No problem IDs provided"
        })));
    }
    
    if body.problem_ids.len() > 50 {
        return Ok(HttpResponse::BadRequest().json(serde_json::json!({
            "error": "Too many problems (max 50 per batch)"
        })));
    }
    
    let provider = body.provider.as_deref().unwrap_or("mistral");
    
    let processor = BatchProcessor::new(
        job_manager.get_ref().clone(),
        Arc::new(db.get_ref().clone()),
        Arc::new(config.get_ref().clone()),
    );
    
    match processor.start_batch_solve(body.problem_ids.clone(), provider).await {
        Ok(job_id) => {
            Ok(HttpResponse::Accepted().json(BatchSolveResponse {
                job_id,
                status: "pending".to_string(),
                message: format!("Batch solve started with {} problems", body.problem_ids.len()),
                total_problems: body.problem_ids.len(),
            }))
        }
        Err(e) => {
            log::error!("Failed to start batch solve: {}", e);
            Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Failed to start batch solve: {}", e)
            })))
        }
    }
}

// === Job Management ===

#[derive(Debug, Serialize)]
pub struct JobStatusResponse {
    pub job_id: String,
    pub status: String,
    pub progress: Option<f32>,
    pub message: Option<String>,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

pub async fn get_job_status(
    path: web::Path<String>,
    job_manager: web::Data<Arc<JobManager>>,
) -> Result<HttpResponse, Error> {
    let job_id = path.into_inner();
    
    match job_manager.get_job(&job_id).await {
        Some(job) => {
            let (status, progress, message, result, error) = match &job.status {
                JobStatus::Pending => ("pending".to_string(), None, None, None, None),
                JobStatus::Running { progress, message } => {
                    ("running".to_string(), Some(*progress), Some(message.clone()), None, None)
                }
                JobStatus::Completed { result } => {
                    ("completed".to_string(), Some(100.0), Some("Done".to_string()), Some(result.clone()), None)
                }
                JobStatus::Failed { error } => {
                    ("failed".to_string(), None, None, None, Some(error.clone()))
                }
                JobStatus::Cancelled => ("cancelled".to_string(), None, None, None, None),
            };
            
            Ok(HttpResponse::Ok().json(JobStatusResponse {
                job_id: job.id,
                status,
                progress,
                message,
                result,
                error,
                created_at: job.created_at.to_rfc3339(),
                updated_at: job.updated_at.to_rfc3339(),
            }))
        }
        None => Ok(HttpResponse::NotFound().json(serde_json::json!({
            "error": "Job not found"
        }))),
    }
}

pub async fn list_jobs(
    job_manager: web::Data<Arc<JobManager>>,
) -> Result<HttpResponse, Error> {
    let jobs = job_manager.list_jobs().await;
    
    let responses: Vec<JobStatusResponse> = jobs.into_iter().map(|job| {
        let (status, progress, message, result, error) = match &job.status {
            JobStatus::Pending => ("pending".to_string(), None, None, None, None),
            JobStatus::Running { progress, message } => {
                ("running".to_string(), Some(*progress), Some(message.clone()), None, None)
            }
            JobStatus::Completed { result } => {
                ("completed".to_string(), Some(100.0), Some("Done".to_string()), Some(result.clone()), None)
            }
            JobStatus::Failed { error } => {
                ("failed".to_string(), None, None, None, Some(error.clone()))
            }
            JobStatus::Cancelled => ("cancelled".to_string(), None, None, None, None),
        };
        
        JobStatusResponse {
            job_id: job.id,
            status,
            progress,
            message,
            result,
            error,
            created_at: job.created_at.to_rfc3339(),
            updated_at: job.updated_at.to_rfc3339(),
        }
    }).collect();
    
    Ok(HttpResponse::Ok().json(responses))
}

pub async fn cancel_job(
    path: web::Path<String>,
    job_manager: web::Data<Arc<JobManager>>,
) -> Result<HttpResponse, Error> {
    let job_id = path.into_inner();
    
    match job_manager.get_job(&job_id).await {
        Some(job) => {
            match job.status {
                JobStatus::Running { .. } | JobStatus::Pending => {
                    job_manager.cancel_job(&job_id).await;
                    Ok(HttpResponse::Ok().json(serde_json::json!({
                        "message": "Job cancelled"
                    })))
                }
                _ => Ok(HttpResponse::BadRequest().json(serde_json::json!({
                    "error": "Job is not running"
                }))),
            }
        }
        None => Ok(HttpResponse::NotFound().json(serde_json::json!({
            "error": "Job not found"
        }))),
    }
}

// === Export ===

#[derive(Debug, Deserialize)]
pub struct ExportRequest {
    pub book_id: String,
    pub format: String, // markdown, latex, json, anki
}

pub async fn export_book(
    body: web::Json<ExportRequest>,
    db: web::Data<Database>,
) -> Result<HttpResponse, Error> {
    use crate::services::export::{Exporter, ExportFormat};
    
    let format = match body.format.as_str() {
        "markdown" | "md" => ExportFormat::Markdown,
        "latex" | "tex" => ExportFormat::Latex,
        "json" => ExportFormat::Json,
        "anki" => ExportFormat::Anki,
        _ => {
            return Ok(HttpResponse::BadRequest().json(serde_json::json!({
                "error": "Invalid format. Use: markdown, latex, json, anki"
            })));
        }
    };
    
    let exporter = Exporter::new(db.get_ref().clone());
    
    match exporter.export_book(&body.book_id, format).await {
        Ok(data) => {
            let filename = format!("{}_export.{}", body.book_id, format.extension());
            
            Ok(HttpResponse::Ok()
                .content_type(format.mime_type())
                .append_header(("Content-Disposition", format!("attachment; filename=\"{}\"", filename)))
                .body(data))
        }
        Err(e) => {
            log::error!("Export failed: {}", e);
            Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Export failed: {}", e)
            })))
        }
    }
}

pub async fn export_chapter(
    path: web::Path<String>,
    query: web::Query<std::collections::HashMap<String, String>>,
    db: web::Data<Database>,
) -> Result<HttpResponse, Error> {
    use crate::services::export::{Exporter, ExportFormat};
    
    let chapter_id = path.into_inner();
    let format_str = query.get("format").map(|s| s.as_str()).unwrap_or("markdown");
    
    let format = match format_str {
        "markdown" | "md" => ExportFormat::Markdown,
        "latex" | "tex" => ExportFormat::Latex,
        "json" => ExportFormat::Json,
        "anki" => ExportFormat::Anki,
        _ => {
            return Ok(HttpResponse::BadRequest().json(serde_json::json!({
                "error": "Invalid format"
            })));
        }
    };
    
    let exporter = Exporter::new(db.get_ref().clone());
    
    match exporter.export_chapter(&chapter_id, format).await {
        Ok(data) => {
            let filename = format!("chapter_{}_export.{}", chapter_id.replace(":", "_"), format.extension());
            
            Ok(HttpResponse::Ok()
                .content_type(format.mime_type())
                .append_header(("Content-Disposition", format!("attachment; filename=\"{}\"", filename)))
                .body(data))
        }
        Err(e) => {
            log::error!("Export failed: {}", e);
            Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Export failed: {}", e)
            })))
        }
    }
}

// === Validation ===

#[derive(Debug, Deserialize)]
pub struct ValidateRequest {
    pub chapter_id: String,
}

#[derive(Debug, Serialize)]
pub struct ValidationResponse {
    pub is_valid: bool,
    pub errors: Vec<ValidationErrorResponse>,
    pub warnings: Vec<ValidationWarningResponse>,
}

#[derive(Debug, Serialize)]
pub struct ValidationErrorResponse {
    pub code: String,
    pub message: String,
    pub problem_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ValidationWarningResponse {
    pub code: String,
    pub message: String,
    pub problem_id: Option<String>,
}

pub async fn validate_chapter(
    body: web::Json<ValidateRequest>,
    db: web::Data<Database>,
) -> Result<HttpResponse, Error> {
    use crate::services::validation::{validate_problem_sequence, validate_problem};
    
    // Get all problems for chapter
    let problems = match db.get_problems_by_chapter(&body.chapter_id).await {
        Ok(p) => p,
        Err(e) => {
            return Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Failed to get problems: {}", e)
            })));
        }
    };
    
    // Validate sequence
    let seq_result = validate_problem_sequence(&problems);
    
    // Validate individual problems
    let mut all_errors = seq_result.errors.clone();
    let mut all_warnings = seq_result.warnings.clone();
    
    for problem in &problems {
        let problem_result = validate_problem(problem);
        all_errors.extend(problem_result.errors);
        all_warnings.extend(problem_result.warnings);
    }
    
    let response = ValidationResponse {
        is_valid: all_errors.is_empty(),
        errors: all_errors.into_iter().map(|e| ValidationErrorResponse {
            code: e.code,
            message: e.message,
            problem_id: e.problem_id,
        }).collect(),
        warnings: all_warnings.into_iter().map(|w| ValidationWarningResponse {
            code: w.code,
            message: w.message,
            problem_id: w.problem_id,
        }).collect(),
    };
    
    Ok(HttpResponse::Ok().json(response))
}

// === Formula Search ===

#[derive(Debug, Deserialize)]
pub struct FormulaSearchRequest {
    pub query: String,
    pub limit: Option<usize>,
}

pub async fn search_by_formula(
    body: web::Json<FormulaSearchRequest>,
    db: web::Data<Database>,
) -> Result<HttpResponse, Error> {
    let limit = body.limit.unwrap_or(20);
    
    // Search in database
    match db.search_by_formula(&body.query, limit).await {
        Ok(problems) => {
            Ok(HttpResponse::Ok().json(serde_json::json!({
                "query": body.query,
                "count": problems.len(),
                "problems": problems
            })))
        }
        Err(e) => {
            log::error!("Formula search failed: {}", e);
            Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Search failed: {}", e)
            })))
        }
    }
}
