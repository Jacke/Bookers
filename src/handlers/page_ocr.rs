use actix_web::{web, Error, HttpResponse};
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::services::database::Database;
use crate::services::ai_parser::HybridParser;
use crate::services::OcrService;
use crate::services::page_parser::{PageContentParser, convert_to_models};
use crate::models::{Problem, Book};

#[derive(Debug, Deserialize)]
pub struct ParseProblemsRequest {
    pub text: String,
    pub book_id: String,
    pub chapter_num: Option<u32>,
    pub page_number: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct CreateProblemsRequest {
    pub text: String,
    pub book_id: String,
    pub chapter_id: String,
    pub chapter_num: u32,
    pub page_number: Option<u32>,
    /// Previous page's last problem number (for cross-page detection)
    pub prev_page_last_problem: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ParsedProblem {
    pub number: String,
    pub content: String,
    pub latex_formulas: Vec<String>,
    pub is_problem: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub_problems: Option<Vec<ParsedProblem>>,
    /// Cross-page flags
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continues_from_prev: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continues_to_next: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct ParseProblemsResponse {
    pub problems: Vec<ParsedProblem>,
    pub total_count: usize,
    pub parser_used: String, // "ai" or "regex"
    pub cross_page_notes: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct CreateProblemsResponse {
    pub created_count: usize,
    pub problems: Vec<String>, // IDs of created problems
    pub cross_page_links: Vec<CrossPageLink>,
}

#[derive(Debug, Serialize)]
pub struct CrossPageLink {
    pub problem_number: String,
    pub from_page: Option<u32>,
    pub to_page: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct PageOcrRequest {
    pub provider: Option<String>, // mistral, mathpix, etc.
}

#[derive(Debug, Serialize)]
pub struct PageOcrResponse {
    pub page: u32,
    pub text: String,
    pub provider: String,
}

/// Get the hybrid parser (AI + regex fallback)
fn get_parser() -> HybridParser {
    let api_key = std::env::var("MISTRAL_API_KEY").ok();
    HybridParser::new(api_key)
}

/// Perform OCR on a specific PDF page
pub async fn ocr_pdf_page(
    path: web::Path<(String, u32)>,
    query: web::Query<PageOcrRequest>,
    config: web::Data<Config>,
) -> Result<HttpResponse, Error> {
    let (filename, page) = path.into_inner();
    let provider = query.provider.as_deref().unwrap_or("mistral");
    
    // Check if preview image exists
    let preview_dir = &config.preview_dir;
    let png_path = preview_dir.join(format!("{}_{}.png", filename, page));
    let jpg_path = preview_dir.join(format!("{}_{}.jpg", filename, page));
    
    let image_path = if png_path.exists() {
        png_path
    } else if jpg_path.exists() {
        jpg_path
    } else {
        return Ok(HttpResponse::NotFound().json(serde_json::json!({
            "error": "Preview image not found. Generate previews first."
        })));
    };
    
    // Run OCR using the shared OCR service (supports provider selection and retries).
    let ocr_service = OcrService::new(config.preview_dir.clone());
    let ocr_result = match ocr_service.run_ocr(&image_path, provider).await {
        Ok(text) => text,
        Err(e) => {
            log::error!("OCR failed: {}", e);
            return Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("OCR failed: {}", e)
            })));
        }
    };
    
    Ok(HttpResponse::Ok().json(PageOcrResponse {
        page,
        text: ocr_result,
        provider: provider.to_string(),
    }))
}

/// Parse problems from OCR text using hybrid AI+regex parser
pub async fn parse_problems_from_text(
    body: web::Json<ParseProblemsRequest>,
) -> Result<HttpResponse, Error> {
    let parser = get_parser();
    let page_number = body.page_number;
    
    // Parse with hybrid parser (AI first, regex fallback)
    match parser.parse_text(&body.book_id, &body.text, page_number).await {
        Ok(result) => {
            let parser_used = if std::env::var("MISTRAL_API_KEY").is_ok() { "ai" } else { "regex" };
            
            // Convert to response format
            let problems: Vec<ParsedProblem> = result.problems.iter().map(|p| {
                convert_ai_problem(p)
            }).collect();
            
            // Generate cross-page notes
            let cross_page_notes = generate_cross_page_notes(&result.problems);
            
            Ok(HttpResponse::Ok().json(ParseProblemsResponse {
                total_count: problems.len(),
                problems,
                parser_used: parser_used.to_string(),
                cross_page_notes: if cross_page_notes.is_empty() { None } else { Some(cross_page_notes) },
            }))
        }
        Err(e) => {
            log::error!("Parsing failed: {}", e);
            Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Parsing failed: {}", e)
            })))
        }
    }
}

/// Create problems from parsed OCR text - uses hybrid parser
/// Deletes ALL old problems on this page before creating new ones
pub async fn create_problems_from_ocr(
    body: web::Json<CreateProblemsRequest>,
    db: web::Data<Database>,
) -> Result<HttpResponse, Error> {
    log::info!("Creating problems for book={}, chapter={}, page={:?}", 
               body.book_id, body.chapter_id, body.page_number);
    
    let parser = get_parser();
    let page_number = body.page_number.unwrap_or(1);
    
    // Parse with hybrid parser
    let result = match parser.parse_text(&body.book_id, &body.text, Some(page_number)).await {
        Ok(r) => {
            log::info!("Parsed {} problems", r.problems.len());
            r
        }
        Err(e) => {
            log::error!("Parsing failed: {}", e);
            return Ok(HttpResponse::BadRequest().json(serde_json::json!({
                "error": format!("Failed to parse text: {}", e)
            })));
        }
    };
    
    if result.problems.is_empty() {
        return Ok(HttpResponse::BadRequest().json(serde_json::json!({
            "error": "No problems found in OCR text"
        })));
    }
    
    // Ensure book exists first
    let book = crate::models::Book {
        id: body.book_id.clone(),
        title: body.book_id.clone(),
        author: None,
        subject: None,
        file_path: format!("resources/{}.pdf", body.book_id),
        total_pages: 0,
        created_at: chrono::Utc::now(),
    };
    
    if let Err(e) = db.create_book(&book).await {
        log::debug!("Book may already exist: {}", e);
    }
    
    // Ensure chapter exists
    let chapter = crate::models::Chapter {
        id: body.chapter_id.clone(),
        book_id: body.book_id.clone(),
        number: body.chapter_num,
        title: format!("Глава {}", body.chapter_num),
        description: None,
        problem_count: 0,
        theory_count: 0,
        created_at: chrono::Utc::now(),
    };
    
    if let Err(e) = db.create_chapter(&chapter).await {
        log::debug!("Chapter may already exist: {}", e);
    }
    
    // Get or create the page
    let page = match db.get_or_create_page(&body.book_id, page_number).await {
        Ok(p) => p,
        Err(e) => {
            log::error!("Failed to get/create page: {}", e);
            return Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Failed to create page: {}", e)
            })));
        }
    };
    
    // DELETE ALL old problems on this page before creating new ones
    let deleted_count = match db.delete_problems_by_page(&page.id).await {
        Ok(count) => {
            if count > 0 {
                log::info!("🗑️ Deleted {} old problems from page {}", count, page.id);
            }
            count
        }
        Err(e) => {
            log::error!("Failed to delete old problems: {}", e);
            0
        }
    };
    
    // Update page with OCR text
    if let Err(e) = db.update_page_ocr(&page.id, &body.text, result.problems.len() as u32).await {
        log::error!("Failed to update page OCR: {}", e);
    }
    
    // Build problems with cross-page detection
    let mut problems_to_create: Vec<Problem> = Vec::new();
    let mut cross_page_links: Vec<CrossPageLink> = Vec::new();
    
    for ai_problem in &result.problems {
        let problem_id = format!("{}:{}:{}", body.book_id, body.chapter_num, ai_problem.number);
        
        // Track cross-page links
        if ai_problem.continues_from_prev || ai_problem.continues_to_next {
            cross_page_links.push(CrossPageLink {
                problem_number: ai_problem.number.clone(),
                from_page: if ai_problem.continues_from_prev { 
                    Some(page_number.saturating_sub(1)) 
                } else { None },
                to_page: if ai_problem.continues_to_next { 
                    Some(page_number + 1) 
                } else { None },
            });
        }
        
        // Create main problem
        let main_problem = Problem {
            id: problem_id.clone(),
            chapter_id: body.chapter_id.clone(),
            page_id: Some(page.id.clone()),
            parent_id: None,
            number: ai_problem.number.clone(),
            display_name: format!("Задача {}", ai_problem.number),
            content: ai_problem.content.clone(),
            latex_formulas: extract_formulas(&ai_problem.content),
            page_number: Some(page_number),
            difficulty: None,
            has_solution: false,
            created_at: chrono::Utc::now(),
            solution: None,
            sub_problems: None,
            continues_from_page: if ai_problem.continues_from_prev { 
                Some(page_number.saturating_sub(1)) 
            } else { None },
            continues_to_page: if ai_problem.continues_to_next { 
                Some(page_number + 1) 
            } else { None },
            is_cross_page: ai_problem.continues_from_prev || ai_problem.continues_to_next,
            is_bookmarked: false,
        };
        
        problems_to_create.push(main_problem);
        
        // Create sub-problems
        for sub in &ai_problem.sub_problems {
            let sub_id = format!("{}:{}", problem_id, sub.letter);
            let sub_problem = Problem {
                id: sub_id,
                chapter_id: body.chapter_id.clone(),
                page_id: Some(page.id.clone()),
                parent_id: Some(problem_id.clone()),
                number: sub.letter.clone(),
                display_name: format!("{})", sub.letter),
                content: sub.content.clone(),
                latex_formulas: extract_formulas(&sub.content),
                page_number: Some(page_number),
                difficulty: None,
                has_solution: false,
                created_at: chrono::Utc::now(),
                solution: None,
                sub_problems: None,
                continues_from_page: None,
                continues_to_page: None,
                is_cross_page: false,
                is_bookmarked: false,
            };
            problems_to_create.push(sub_problem);
        }
    }
    
    // Save to database
    log::info!("Saving {} problems to database", problems_to_create.len());
    match db.create_or_update_problems(&problems_to_create).await {
        Ok(count) => {
            log::info!("Successfully created {} problems", count);
            let problem_ids: Vec<String> = problems_to_create.iter()
                .filter(|p| p.parent_id.is_none()) // Only main problems
                .map(|p| p.id.clone())
                .collect();
            
            Ok(HttpResponse::Ok().json(serde_json::json!({
                "deleted_count": deleted_count,
                "created_count": count,
                "page_id": page.id,
                "page_number": page_number,
                "problems": problem_ids,
                "cross_page_links": cross_page_links,
                "message": format!("Replaced: deleted {}, created {}", deleted_count, count),
            })))
        }
        Err(e) => {
            log::error!("Failed to create problems: {}", e);
            Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Failed to create problems: {}", e)
            })))
        }
    }
}

/// Get existing OCR text for a page
pub async fn get_page_ocr(
    path: web::Path<(String, u32)>,
    db: web::Data<Database>,
) -> Result<HttpResponse, Error> {
    let (book_id, page_number) = path.into_inner();
    
    match db.get_page(&book_id, page_number).await {
        Ok(Some(page)) => {
            Ok(HttpResponse::Ok().json(serde_json::json!({
                "page_id": page.id,
                "page_number": page.page_number,
                "has_ocr": page.ocr_text.is_some(),
                "ocr_text": page.ocr_text.unwrap_or_default(),
                "has_problems": page.has_problems,
                "problem_count": page.problem_count,
            })))
        }
        // First visit to a page may have no OCR record yet; return empty state instead of 404.
        Ok(None) => Ok(HttpResponse::Ok().json(serde_json::json!({
            "page_id": format!("{}:page:{}", book_id, page_number),
            "page_number": page_number,
            "has_ocr": false,
            "ocr_text": "",
            "has_problems": false,
            "problem_count": 0,
        }))),
        Err(e) => {
            log::error!("Failed to get page OCR: {}", e);
            Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Failed to get page OCR: {}", e)
            })))
        }
    }
}

/// Get problems by page ID
pub async fn get_problems_by_page(
    path: web::Path<String>,
    db: web::Data<Database>,
) -> Result<HttpResponse, Error> {
    let page_id = path.into_inner();
    
    match db.get_problems_by_page(&page_id).await {
        Ok(problems) => Ok(HttpResponse::Ok().json(problems)),
        Err(e) => {
            log::error!("Failed to get problems by page: {}", e);
            Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Failed to get problems: {}", e)
            })))
        }
    }
}

// Helper functions

fn convert_ai_problem(p: &crate::services::ai_parser::ParsedProblem) -> ParsedProblem {
    let latex_formulas = extract_formulas(&p.content);
    
    let sub_problems = if p.sub_problems.is_empty() {
        None
    } else {
        Some(p.sub_problems.iter().map(|s| {
            ParsedProblem {
                number: s.letter.clone(),
                content: s.content.clone(),
                latex_formulas: extract_formulas(&s.content),
                is_problem: true,
                sub_problems: None,
                continues_from_prev: None,
                continues_to_next: None,
            }
        }).collect())
    };
    
    ParsedProblem {
        number: p.number.clone(),
        content: p.content.clone(),
        latex_formulas,
        is_problem: true,
        sub_problems,
        continues_from_prev: Some(p.continues_from_prev),
        continues_to_next: Some(p.continues_to_next),
    }
}

fn generate_cross_page_notes(problems: &[crate::services::ai_parser::ParsedProblem]) -> Vec<String> {
    let mut notes = Vec::new();
    
    for p in problems {
        if p.continues_from_prev {
            notes.push(format!("Задача {} продолжается с предыдущей страницы", p.number));
        }
        if p.continues_to_next {
            notes.push(format!("Задача {} продолжается на следующей странице", p.number));
        }
    }
    
    notes
}

fn extract_formulas(text: &str) -> Vec<String> {
    let mut formulas = Vec::new();
    let re = regex::Regex::new(r"\$([^$]+)\$").unwrap();
    for cap in re.captures_iter(text) {
        formulas.push(cap[1].to_string());
    }
    formulas
}

// === Full Page Content Parsing ===

#[derive(Debug, Deserialize)]
pub struct ParseFullPageRequest {
    pub text: String,
    pub book_id: String,
    pub chapter_num: u32,
    pub page_number: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct ParseFullPageResponse {
    pub metadata: serde_json::Value,
    pub elements: Vec<serde_json::Value>,
    pub stats: serde_json::Value,
    pub problems_created: usize,
    pub theory_created: usize,
}

/// Parse full page content including theory, examples, figures, problems
pub async fn parse_full_page(
    body: web::Json<ParseFullPageRequest>,
    db: web::Data<Database>,
) -> Result<HttpResponse, Error> {
    let api_key = std::env::var("MISTRAL_API_KEY").ok();
    let parser = PageContentParser::new(api_key);
    
    // Parse the page
    let result = match parser.parse_page(&body.text, body.page_number).await {
        Ok(r) => r,
        Err(e) => {
            log::error!("Full page parsing failed: {}", e);
            return Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Parsing failed: {}", e)
            })));
        }
    };
    
    // Convert to database models
    let (problems, theories) = convert_to_models(result.clone(), &body.book_id, body.chapter_num);
    
    // Ensure book and chapter exist
    let book = Book {
        id: body.book_id.clone(),
        title: body.book_id.clone(),
        author: None,
        subject: None,
        file_path: format!("resources/{}.pdf", body.book_id),
        total_pages: 0,
        created_at: chrono::Utc::now(),
    };
    let _ = db.create_book(&book).await;
    
    let chapter = crate::models::Chapter {
        id: format!("{}:{}", body.book_id, body.chapter_num),
        book_id: body.book_id.clone(),
        number: body.chapter_num,
        title: format!("Глава {}", body.chapter_num),
        description: result.metadata.chapter_title.clone(),
        problem_count: 0,
        theory_count: 0,
        created_at: chrono::Utc::now(),
    };
    let _ = db.create_chapter(&chapter).await;
    
    // Save problems
    let mut problems_created = 0;
    if !problems.is_empty() {
        match db.create_or_update_problems(&problems).await {
            Ok(count) => problems_created = count,
            Err(e) => log::error!("Failed to save problems: {}", e),
        }
    }
    
    // Save theory blocks
    let mut theory_created = 0;
    for theory in &theories {
        match db.create_theory_block(theory).await {
            Ok(_) => theory_created += 1,
            Err(e) => log::error!("Failed to save theory: {}", e),
        }
    }
    
    Ok(HttpResponse::Ok().json(serde_json::json!({
        "metadata": result.metadata,
        "elements": result.elements,
        "stats": result.stats,
        "problems_created": problems_created,
        "theory_created": theory_created,
    })))
}
