use actix_web::{web, Error, HttpResponse};
use tera::{Context, Tera};

use crate::services::database::Database;
use crate::services::parser::TextbookParser;

/// View chapter problems page
pub async fn view_chapter(
    path: web::Path<String>,
    tmpl: web::Data<Tera>,
    db: web::Data<Database>,
) -> Result<HttpResponse, Error> {
    let chapter_id = path.into_inner();
    
    // Get chapter
    let chapter = match db.get_chapter(&chapter_id).await {
        Ok(Some(c)) => c,
        Ok(None) => return Ok(HttpResponse::NotFound().body("Chapter not found")),
        Err(e) => {
            log::error!("Database error: {}", e);
            return Ok(HttpResponse::InternalServerError().body("Database error"));
        }
    };
    
    // Get problems
    let problems = db.get_problems_by_chapter(&chapter_id).await.map_err(|e| {
        log::error!("Failed to get problems: {}", e);
        actix_web::error::ErrorInternalServerError(e)
    })?;
    
    // Get theory blocks
    let theory_blocks = db.get_theory_blocks_by_chapter(&chapter_id).await.map_err(|e| {
        log::error!("Failed to get theory: {}", e);
        actix_web::error::ErrorInternalServerError(e)
    })?;
    
    // Count solved problems
    let solved_count = problems.iter().filter(|p| p.has_solution).count();
    
    // Get book info
    let book = db.get_book(&chapter.book_id).await.map_err(|e| {
        log::error!("Failed to get book: {}", e);
        actix_web::error::ErrorInternalServerError(e)
    })?.unwrap_or_else(|| crate::models::Book {
        id: chapter.book_id.clone(),
        title: "Unknown Book".to_string(),
        author: None,
        subject: None,
        file_path: String::new(),
        total_pages: 0,
        created_at: chrono::Utc::now(),
    });
    
    let mut context = Context::new();
    context.insert("chapter", &chapter);
    context.insert("problems", &problems);
    context.insert("theory_blocks", &theory_blocks);
    context.insert("solved_count", &solved_count);
    context.insert("book", &book);
    context.insert("book_id", &book.id);
    context.insert("book_title", &book.title);
    
    let rendered = tmpl.render("textbook/chapter_problems.html", &context).map_err(|e| {
        log::error!("Template error: {}", e);
        actix_web::error::ErrorInternalServerError(e)
    })?;
    
    Ok(HttpResponse::Ok().content_type("text/html").body(rendered))
}

/// View single problem page
pub async fn view_problem(
    path: web::Path<String>,
    tmpl: web::Data<Tera>,
    db: web::Data<Database>,
) -> Result<HttpResponse, Error> {
    let problem_id = path.into_inner();
    
    // Get problem with sub-problems
    let problem = match db.get_problem_with_subs(&problem_id).await {
        Ok(Some(p)) => p,
        Ok(None) => return Ok(HttpResponse::NotFound().body("Problem not found")),
        Err(e) => {
            log::error!("Database error: {}", e);
            return Ok(HttpResponse::InternalServerError().body("Database error"));
        }
    };
    
    // Get chapter
    let chapter = db.get_chapter(&problem.chapter_id).await.map_err(|e| {
        log::error!("Failed to get chapter: {}", e);
        actix_web::error::ErrorInternalServerError(e)
    })?.unwrap_or_else(|| crate::models::Chapter {
        id: problem.chapter_id.clone(),
        book_id: "unknown".to_string(),
        number: 0,
        title: "Unknown Chapter".to_string(),
        description: None,
        problem_count: 0,
        theory_count: 0,
        created_at: chrono::Utc::now(),
    });
    
    // Get book
    let book = db.get_book(&chapter.book_id).await.map_err(|e| {
        log::error!("Failed to get book: {}", e);
        actix_web::error::ErrorInternalServerError(e)
    })?.unwrap_or_else(|| crate::models::Book {
        id: chapter.book_id.clone(),
        title: "Unknown Book".to_string(),
        author: None,
        subject: None,
        file_path: String::new(),
        total_pages: 0,
        created_at: chrono::Utc::now(),
    });
    
    // Load parent problem if this is a sub-problem
    let parent_problem = if let Some(ref parent_id) = problem.parent_id {
        db.get_problem(parent_id).await.ok().flatten()
    } else {
        None
    };
    
    let mut context = Context::new();
    context.insert("problem", &problem);
    context.insert("parent_problem", &parent_problem);
    context.insert("chapter", &chapter);
    context.insert("book", &book);
    context.insert("book_id", &book.id);
    context.insert("book_title", &book.title);
    
    let rendered = tmpl.render("textbook/problem_view.html", &context).map_err(|e| {
        log::error!("Template error: {}", e);
        actix_web::error::ErrorInternalServerError(e)
    })?;
    
    Ok(HttpResponse::Ok().content_type("text/html").body(rendered))
}

/// Parse and import textbook from OCR text
pub async fn import_textbook(
    body: web::Json<ImportRequest>,
    db: web::Data<Database>,
) -> Result<HttpResponse, Error> {
    let parser = TextbookParser::new();
    
    // Parse text
    let result = parser.parse(&body.text, &body.book_id, body.chapter_num);
    
    // Create book first (if not exists)
    let book = crate::models::Book {
        id: body.book_id.clone(),
        title: format!("Book {}", body.book_id),
        author: None,
        subject: Some("Mathematics".to_string()),
        file_path: String::new(),
        total_pages: 0,
        created_at: chrono::Utc::now(),
    };
    
    if let Err(e) = db.create_book(&book).await {
        log::warn!("Failed to create book (may already exist): {}", e);
    }
    
    // Create or update chapter
    let chapter_id = format!("{}:{}", body.book_id, body.chapter_num);
    let chapter = crate::models::Chapter {
        id: chapter_id.clone(),
        book_id: body.book_id.clone(),
        number: body.chapter_num,
        title: body.chapter_title.clone(),
        description: None,
        problem_count: result.problems.len() as u32,
        theory_count: result.theory_blocks.len() as u32,
        created_at: chrono::Utc::now(),
    };
    
    if let Err(e) = db.create_chapter(&chapter).await {
        log::error!("Failed to create chapter: {}", e);
        return Ok(HttpResponse::InternalServerError().json(serde_json::json!({
            "error": format!("Failed to create chapter: {}", e)
        })));
    }
    
    // Save problems
    for problem in &result.problems {
        if let Err(e) = db.create_problem(problem).await {
            log::error!("Failed to create problem: {}", e);
        }
    }
    
    // Save theory blocks
    for theory in &result.theory_blocks {
        if let Err(e) = db.create_theory_block(theory).await {
            log::error!("Failed to create theory block: {}", e);
        }
    }
    
    Ok(HttpResponse::Ok().json(serde_json::json!({
        "chapter_id": chapter_id,
        "problems_imported": result.problems.len(),
        "theory_blocks_imported": result.theory_blocks.len(),
    })))
}

/// View book pages (page browser) - shows ALL pages from PDF
pub async fn view_book_pages(
    path: web::Path<String>,
    tmpl: web::Data<Tera>,
    db: web::Data<Database>,
    file_service: web::Data<crate::services::FileService>,
) -> Result<HttpResponse, Error> {
    let book_id = path.into_inner();
    
    // Get book
    let book = match db.get_book(&book_id).await {
        Ok(Some(b)) => b,
        Ok(None) => return Ok(HttpResponse::NotFound().body("Book not found")),
        Err(e) => {
            log::error!("Database error: {}", e);
            return Ok(HttpResponse::InternalServerError().body("Database error"));
        }
    };
    
    // Get pages with OCR data (as map for quick lookup)
    let pages_with_ocr = db.get_pages_by_book(&book_id).await.map_err(|e| {
        log::error!("Failed to get pages: {}", e);
        actix_web::error::ErrorInternalServerError(e)
    })?;
    
    let ocr_pages_map: std::collections::HashMap<u32, crate::models::Page> = pages_with_ocr
        .into_iter()
        .map(|p| (p.page_number, p))
        .collect();
    
    // Get total pages from PDF metadata
    let total_pages = match file_service.get_pdf_page_count(&format!("{}.pdf", book_id)) {
        Ok(count) => count,
        Err(e) => {
            log::warn!("Failed to get PDF page count: {}, using default 100", e);
            100
        }
    };
    
    // Build list of ALL pages (1..total_pages)
    let mut all_pages = Vec::new();
    for page_num in 1..=total_pages {
        if let Some(page) = ocr_pages_map.get(&page_num) {
            all_pages.push(page.clone());
        } else {
            // Create empty page entry
            all_pages.push(crate::models::Page {
                id: format!("{}:page:{}", book_id, page_num),
                book_id: book_id.clone(),
                page_number: page_num,
                ocr_text: None,
                has_problems: false,
                problem_count: 0,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            });
        }
    }
    
    let mut context = Context::new();
    context.insert("book", &book);
    context.insert("pages", &all_pages);
    context.insert("total_pages", &total_pages);
    context.insert("pages_with_ocr", &ocr_pages_map.len());
    
    let rendered = tmpl.render("textbook/page_browser.html", &context).map_err(|e| {
        log::error!("Template error: {}", e);
        actix_web::error::ErrorInternalServerError(e)
    })?;
    
    Ok(HttpResponse::Ok().content_type("text/html").body(rendered))
}

/// View specific page with OCR and problems
pub async fn view_page(
    path: web::Path<(String, u32)>,
    tmpl: web::Data<Tera>,
    db: web::Data<Database>,
) -> Result<HttpResponse, Error> {
    let (book_id, page_number) = path.into_inner();
    
    // Get book
    let book = match db.get_book(&book_id).await {
        Ok(Some(b)) => b,
        Ok(None) => return Ok(HttpResponse::NotFound().body("Book not found")),
        Err(e) => {
            log::error!("Database error: {}", e);
            return Ok(HttpResponse::InternalServerError().body("Database error"));
        }
    };
    
    // Get page info
    let page = db.get_page(&book_id, page_number).await.map_err(|e| {
        log::error!("Failed to get page: {}", e);
        actix_web::error::ErrorInternalServerError(e)
    })?;
    
    // Get problems on this page (with sub-problems)
    let page_id = format!("{}:page:{}", book_id, page_number);
    let mut problems = db.get_problems_by_page(&page_id).await.map_err(|e| {
        log::error!("Failed to get problems: {}", e);
        actix_web::error::ErrorInternalServerError(e)
    })?;
    
    // Load sub-problems for each problem
    for problem in &mut problems {
        let subs = db.get_sub_problems(&problem.id).await.map_err(|e| {
            log::error!("Failed to get sub-problems: {}", e);
            actix_web::error::ErrorInternalServerError(e)
        })?;
        if !subs.is_empty() {
            problem.sub_problems = Some(subs);
        }
    }
    
    // Get preview image path
    let preview_path = format!("/preview_image/{}.pdf/{}", book_id, page_number);
    
    let mut context = Context::new();
    context.insert("book", &book);
    context.insert("book_id", &book_id);
    context.insert("page_number", &page_number);
    context.insert("page", &page);
    context.insert("problems", &problems);
    context.insert("preview_path", &preview_path);
    
    let rendered = tmpl.render("textbook/page_view.html", &context).map_err(|e| {
        log::error!("Template error: {}", e);
        actix_web::error::ErrorInternalServerError(e)
    })?;
    
    Ok(HttpResponse::Ok().content_type("text/html").body(rendered))
}

#[derive(Debug, serde::Deserialize)]
pub struct ImportRequest {
    pub book_id: String,
    pub chapter_num: u32,
    pub chapter_title: String,
    pub text: String,
}
