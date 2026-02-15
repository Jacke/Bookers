use actix_web::{web, Error, HttpResponse};
use serde::Deserialize;

use crate::models::{SolveRequest, SolutionResponse};
use crate::services::database::Database;
use crate::services::ai_solver::AISolver;
use crate::config::Config;

/// Get all problems for a chapter
pub async fn get_chapter_problems(
    path: web::Path<String>,
    db: web::Data<Database>,
) -> Result<HttpResponse, Error> {
    let chapter_id = path.into_inner();
    
    match db.get_problems_by_chapter(&chapter_id).await {
        Ok(problems) => Ok(HttpResponse::Ok().json(problems)),
        Err(e) => {
            log::error!("Failed to get problems: {}", e);
            Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Failed to get problems: {}", e)
            })))
        }
    }
}

/// Get single problem with optional solution
pub async fn get_problem(
    path: web::Path<String>,
    query: web::Query<GetProblemQuery>,
    db: web::Data<Database>,
) -> Result<HttpResponse, Error> {
    let problem_id = path.into_inner();
    
    let mut problem = match db.get_problem_with_subs(&problem_id).await {
        Ok(Some(p)) => p,
        Ok(None) => return Ok(HttpResponse::NotFound().json(serde_json::json!({
            "error": "Problem not found"
        }))),
        Err(e) => {
            log::error!("Failed to get problem: {}", e);
            return Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Failed to get problem: {}", e)
            })));
        }
    };

    // Load solution if requested
    if query.with_solution.unwrap_or(false) {
        let solutions = db.get_solutions_by_problem(&problem_id).await.map_err(|e| {
            log::error!("Failed to get solutions: {}", e);
            actix_web::error::ErrorInternalServerError(e)
        })?;
        
        // Use first solution (most recent)
        if let Some(solution) = solutions.into_iter().next() {
            problem.solution = Some(solution);
        }
    }

    Ok(HttpResponse::Ok().json(problem))
}

#[derive(Debug, Deserialize)]
pub struct GetProblemQuery {
    with_solution: Option<bool>,
}

/// Generate or retrieve solution for a problem
pub async fn solve_problem(
    path: web::Path<String>,
    body: web::Json<SolveRequest>,
    db: web::Data<Database>,
    config: web::Data<Config>,
) -> Result<HttpResponse, Error> {
    let problem_id = path.into_inner();
    
    // Get problem
    let problem = match db.get_problem(&problem_id).await {
        Ok(Some(p)) => p,
        Ok(None) => return Ok(HttpResponse::NotFound().json(serde_json::json!({
            "error": "Problem not found"
        }))),
        Err(e) => {
            log::error!("Failed to get problem: {}", e);
            return Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Failed to get problem: {}", e)
            })));
        }
    };

    // Check for existing solution if not forcing regeneration
    if !body.force_regenerate.unwrap_or(false) {
        let provider = body.provider.as_deref().unwrap_or("claude");
        if let Ok(Some(existing)) = db.get_solution(&problem_id, provider).await {
            return Ok(HttpResponse::Ok().json(SolutionResponse {
                problem,
                solution: existing,
                generation_time_ms: 0,
            }));
        }
    }

    // Get theory context for better solutions
    let theory_context = db.get_theory_blocks_by_chapter(&problem.chapter_id)
        .await
        .ok()
        .map(|blocks| {
            blocks.iter()
                .map(|t| t.content.clone())
                .collect::<Vec<_>>()
                .join("\n\n")
        })
        .unwrap_or_default();

    // Generate solution
    let start_time = std::time::Instant::now();
    let solver = match AISolver::new(&config) {
        Ok(s) => s,
        Err(e) => {
            return Ok(HttpResponse::ServiceUnavailable().json(serde_json::json!({
                "error": format!("AI solver not available: {}", e)
            })));
        }
    };

    let solution = match solver.solve(
        &problem,
        body.provider.as_deref(),
        if theory_context.is_empty() { None } else { Some(&theory_context) }
    ).await {
        Ok(s) => s,
        Err(e) => {
            log::error!("Failed to generate solution: {}", e);
            return Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Failed to generate solution: {}", e)
            })));
        }
    };

    // Save solution to database
    if let Err(e) = db.create_or_update_solution(&solution).await {
        log::error!("Failed to save solution: {}", e);
    }

    let generation_time_ms = start_time.elapsed().as_millis() as u64;

    Ok(HttpResponse::Ok().json(SolutionResponse {
        problem,
        solution,
        generation_time_ms,
    }))
}

/// Save or update solution manually
pub async fn save_solution(
    path: web::Path<String>,
    body: web::Json<SaveSolutionRequest>,
    db: web::Data<Database>,
) -> Result<HttpResponse, Error> {
    let problem_id = path.into_inner();
    
    // Verify problem exists
    if db.get_problem(&problem_id).await.map_err(|e| {
        log::error!("Database error: {}", e);
        actix_web::error::ErrorInternalServerError(e)
    })?.is_none() {
        return Ok(HttpResponse::NotFound().json(serde_json::json!({
            "error": "Problem not found"
        })));
    }

    let solution = crate::models::Solution {
        id: crate::models::Solution::generate_id(&problem_id),
        problem_id,
        provider: body.provider.clone().unwrap_or_else(|| "manual".to_string()),
        content: body.content.clone(),
        latex_formulas: extract_latex(&body.content),
        is_verified: body.is_verified.unwrap_or(false),
        rating: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    match db.create_or_update_solution(&solution).await {
        Ok(_) => Ok(HttpResponse::Ok().json(solution)),
        Err(e) => {
            log::error!("Failed to save solution: {}", e);
            Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Failed to save solution: {}", e)
            })))
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct SaveSolutionRequest {
    pub content: String,
    pub provider: Option<String>,
    pub is_verified: Option<bool>,
}

/// Rate a solution
pub async fn rate_solution(
    path: web::Path<(String, String)>,
    body: web::Json<RateRequest>,
    db: web::Data<Database>,
) -> Result<HttpResponse, Error> {
    let (_problem_id, solution_id) = path.into_inner();
    
    match db.rate_solution(&solution_id, body.rating).await {
        Ok(_) => Ok(HttpResponse::Ok().json(serde_json::json!({
            "success": true
        }))),
        Err(e) => {
            log::error!("Failed to rate solution: {}", e);
            Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Failed to rate solution: {}", e)
            })))
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct RateRequest {
    pub rating: u8, // 1-5
}

#[derive(Debug, Deserialize)]
pub struct HintRequest {
    pub hint_level: Option<u8>, // 1-3 (1=minimal, 2=moderate, 3=strong)
    pub provider: Option<String>,
}

/// Generate hint for a problem
pub async fn hint_problem(
    path: web::Path<String>,
    body: web::Json<HintRequest>,
    db: web::Data<Database>,
    config: web::Data<Config>,
) -> Result<HttpResponse, Error> {
    let problem_id = path.into_inner();
    
    // Get problem
    let problem = match db.get_problem(&problem_id).await {
        Ok(Some(p)) => p,
        Ok(None) => return Ok(HttpResponse::NotFound().json(serde_json::json!({
            "error": "Problem not found"
        }))),
        Err(e) => {
            log::error!("Failed to get problem: {}", e);
            return Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Failed to get problem: {}", e)
            })));
        }
    };

    // Get theory context
    let theory_context = db.get_theory_blocks_by_chapter(&problem.chapter_id)
        .await
        .ok()
        .map(|blocks| {
            blocks.iter()
                .map(|t| t.content.clone())
                .collect::<Vec<_>>()
                .join("\n\n")
        })
        .unwrap_or_default();

    // Generate hint
    let solver = match AISolver::new(&config) {
        Ok(s) => s,
        Err(e) => {
            return Ok(HttpResponse::ServiceUnavailable().json(serde_json::json!({
                "error": format!("AI solver not available: {}", e)
            })));
        }
    };

    let hint_level = body.hint_level.unwrap_or(2).min(3).max(1);
    
    let hint = match solver.hint(
        &problem,
        body.provider.as_deref(),
        if theory_context.is_empty() { None } else { Some(&theory_context) },
        hint_level,
    ).await {
        Ok(h) => h,
        Err(e) => {
            log::error!("Failed to generate hint: {}", e);
            return Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Failed to generate hint: {}", e)
            })));
        }
    };

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "problem_id": problem_id,
        "hint": hint,
        "hint_level": hint_level,
    })))
}

/// Add problem to bookmarks
pub async fn add_bookmark(
    path: web::Path<String>,
    db: web::Data<Database>,
) -> Result<HttpResponse, Error> {
    let problem_id = path.into_inner();
    
    match db.add_bookmark(&problem_id).await {
        Ok(_) => Ok(HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "problem_id": problem_id,
        }))),
        Err(e) => {
            log::error!("Failed to add bookmark: {}", e);
            Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Failed to add bookmark: {}", e)
            })))
        }
    }
}

/// Remove problem from bookmarks
pub async fn remove_bookmark(
    path: web::Path<String>,
    db: web::Data<Database>,
) -> Result<HttpResponse, Error> {
    let problem_id = path.into_inner();
    
    match db.remove_bookmark(&problem_id).await {
        Ok(_) => Ok(HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "problem_id": problem_id,
        }))),
        Err(e) => {
            log::error!("Failed to remove bookmark: {}", e);
            Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Failed to remove bookmark: {}", e)
            })))
        }
    }
}

/// List all bookmarked problems
pub async fn list_bookmarks(
    db: web::Data<Database>,
) -> Result<HttpResponse, Error> {
    match db.get_bookmarked_problems().await {
        Ok(problems) => Ok(HttpResponse::Ok().json(problems)),
        Err(e) => {
            log::error!("Failed to list bookmarks: {}", e);
            Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Failed to list bookmarks: {}", e)
            })))
        }
    }
}

/// Get theory blocks for a chapter
pub async fn get_chapter_theory(
    path: web::Path<String>,
    db: web::Data<Database>,
) -> Result<HttpResponse, Error> {
    let chapter_id = path.into_inner();
    
    match db.get_theory_blocks_by_chapter(&chapter_id).await {
        Ok(theory) => Ok(HttpResponse::Ok().json(theory)),
        Err(e) => {
            log::error!("Failed to get theory: {}", e);
            Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Failed to get theory: {}", e)
            })))
        }
    }
}

/// Update problem content (e.g., from OCR import)
pub async fn update_problem(
    path: web::Path<String>,
    body: web::Json<UpdateProblemRequest>,
    db: web::Data<Database>,
) -> Result<HttpResponse, Error> {
    let problem_id = path.into_inner();
    
    // Verify problem exists
    if db.get_problem(&problem_id).await.map_err(|e| {
        log::error!("Database error: {}", e);
        actix_web::error::ErrorInternalServerError(e)
    })?.is_none() {
        return Ok(HttpResponse::NotFound().json(serde_json::json!({
            "error": "Problem not found"
        })));
    }

    // Extract LaTeX formulas from content
    let latex_formulas = extract_latex(&body.content);

    match db.update_problem_content(&problem_id, &body.content, latex_formulas).await {
        Ok(_) => Ok(HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "message": "Problem updated successfully"
        }))),
        Err(e) => {
            log::error!("Failed to update problem: {}", e);
            Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Failed to update problem: {}", e)
            })))
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct UpdateProblemRequest {
    pub content: String,
}

/// Helper function to extract LaTeX formulas
fn extract_latex(text: &str) -> Vec<String> {
    let mut formulas = Vec::new();
    let inline_re = regex::Regex::new(r"\$([^$]+)\$").unwrap();
    let display_re = regex::Regex::new(r"\$\$([^$]+)\$\$").unwrap();

    for cap in inline_re.captures_iter(text) {
        formulas.push(cap[1].to_string());
    }
    for cap in display_re.captures_iter(text) {
        formulas.push(cap[1].to_string());
    }

    formulas
}
