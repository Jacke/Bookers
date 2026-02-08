use actix_web::{web, Error, HttpResponse};
use serde::{Deserialize, Serialize};

use crate::services::database::Database;
use crate::services::toc_detector::{TocDetector, SmartImporter};
use crate::services::knowledge_graph::{KnowledgeGraphBuilder};
use crate::services::auto_tagger::AutoTagger;
use crate::services::similarity::{SimilarityDetector, ProblemRecommender};

// === TOC Detection ===

#[derive(Debug, Deserialize)]
pub struct TocDetectRequest {
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct TocDetectResponse {
    pub detected: bool,
    pub confidence: f32,
    pub entries: Vec<TocEntryResponse>,
}

#[derive(Debug, Serialize)]
pub struct TocEntryResponse {
    pub number: u32,
    pub title: String,
    pub page_number: Option<u32>,
}

pub async fn detect_toc(
    body: web::Json<TocDetectRequest>,
) -> Result<HttpResponse, Error> {
    let detector = TocDetector::new();
    
    match detector.detect_toc(&body.text) {
        Some(toc) => {
            let entries: Vec<_> = toc.entries.into_iter().map(|e| TocEntryResponse {
                number: e.number,
                title: e.title,
                page_number: e.page_number,
            }).collect();

            Ok(HttpResponse::Ok().json(TocDetectResponse {
                detected: !entries.is_empty(),
                confidence: toc.confidence,
                entries,
            }))
        }
        None => Ok(HttpResponse::Ok().json(TocDetectResponse {
            detected: false,
            confidence: 0.0,
            entries: vec![],
        })),
    }
}

// === Smart Import ===

#[derive(Debug, Deserialize)]
pub struct SmartImportRequest {
    pub book_id: String,
    pub title: String,
    pub total_pages: u32,
    pub toc_page_ocr: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SmartImportResponse {
    pub book_id: String,
    pub chapters_created: usize,
    pub detection_source: String,
    pub chapter_titles: Vec<String>,
}

pub async fn smart_import_book(
    body: web::Json<SmartImportRequest>,
    db: web::Data<Database>,
) -> Result<HttpResponse, Error> {
    let importer = SmartImporter::new();
    
    match importer.import_book_with_chapters(
        &db,
        &body.book_id,
        &body.title,
        body.total_pages,
        body.toc_page_ocr.as_deref(),
    ).await {
        Ok(result) => {
            let chapter_titles: Vec<_> = result.chapters.iter()
                .map(|c| format!("{}. {}", c.number, c.title))
                .collect();

            Ok(HttpResponse::Ok().json(SmartImportResponse {
                book_id: result.book.id,
                chapters_created: result.chapters.len(),
                detection_source: result.detection_source,
                chapter_titles,
            }))
        }
        Err(e) => {
            log::error!("Smart import failed: {}", e);
            Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Import failed: {}", e)
            })))
        }
    }
}

// === Knowledge Graph ===

#[derive(Debug, Deserialize)]
pub struct GraphBuildRequest {
    pub chapter_id: String,
}

pub async fn build_knowledge_graph(
    body: web::Json<GraphBuildRequest>,
    db: web::Data<Database>,
) -> Result<HttpResponse, Error> {
    // Get chapter info
    let chapter = match db.get_chapter(&body.chapter_id).await {
        Ok(Some(c)) => c,
        _ => {
            return Ok(HttpResponse::NotFound().json(serde_json::json!({
                "error": "Chapter not found"
            })));
        }
    };

    // Get all problems for chapter
    let problems = match db.get_problems_by_chapter(&body.chapter_id).await {
        Ok(p) => p,
        Err(e) => {
            return Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Failed to get problems: {}", e)
            })));
        }
    };

    // Build graph
    let mut builder = KnowledgeGraphBuilder::new();

    // Add chapter node
    builder.add_chapter(&chapter.id, &chapter.title, problems.len() as u32);

    // Add problems
    for problem in &problems {
        builder.add_problem(problem);
    }

    // Build similarity edges
    builder.build_similarity_edges(0.3);

    // Build graph with layout
    let graph = builder.build();

    Ok(HttpResponse::Ok().json(graph))
}

// === Auto-tagging ===

#[derive(Debug, Deserialize)]
pub struct AutoTagRequest {
    pub problem_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct AutoTagResponse {
    pub tagged: usize,
    pub results: Vec<TagResultResponse>,
}

#[derive(Debug, Serialize)]
pub struct TagResultResponse {
    pub problem_id: String,
    pub tags: Vec<String>,
    pub difficulty: Option<u8>,
    pub confidence: f32,
}

pub async fn auto_tag_problems(
    body: web::Json<AutoTagRequest>,
    db: web::Data<Database>,
) -> Result<HttpResponse, Error> {
    let api_key = std::env::var("MISTRAL_API_KEY").ok();
    let tagger = AutoTagger::new(api_key);

    // Get problems
    let mut problems = Vec::new();
    for id in &body.problem_ids {
        if let Ok(Some(problem)) = db.get_problem(id).await {
            problems.push(problem);
        }
    }

    // Tag problems
    let tagged_results = tagger.tag_problems(&problems).await;

    // Update database with tags (store in problem content as metadata)
    for result in &tagged_results {
        if let Ok(Some(mut problem)) = db.get_problem(&result.problem_id).await {
            problem.difficulty = result.difficulty;
            // Could store tags in a separate table or as JSON in the problem
        }
    }

    let results: Vec<_> = tagged_results.into_iter().map(|r| TagResultResponse {
        problem_id: r.problem_id,
        tags: r.tags.into_iter().map(|t| t.name).collect(),
        difficulty: r.difficulty,
        confidence: r.confidence,
    }).collect();

    Ok(HttpResponse::Ok().json(AutoTagResponse {
        tagged: results.len(),
        results,
    }))
}

// === Similar Problems ===

#[derive(Debug, Deserialize)]
pub struct SimilarRequest {
    pub problem_id: String,
    pub top_k: Option<usize>,
    pub chapter_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SimilarResponse {
    pub problem_id: String,
    pub similar_problems: Vec<SimilarProblemResponse>,
}

#[derive(Debug, Serialize)]
pub struct SimilarProblemResponse {
    pub problem_id: String,
    pub problem_number: String,
    pub similarity: f64,
    pub match_type: String,
    pub reason: String,
}

pub async fn find_similar_problems(
    body: web::Json<SimilarRequest>,
    db: web::Data<Database>,
) -> Result<HttpResponse, Error> {
    let top_k = body.top_k.unwrap_or(5);

    // Get source problem
    let source = match db.get_problem(&body.problem_id).await {
        Ok(Some(p)) => p,
        _ => {
            return Ok(HttpResponse::NotFound().json(serde_json::json!({
                "error": "Problem not found"
            })));
        }
    };

    // Get candidate problems
    let candidates = if let Some(ref chapter_id) = body.chapter_id {
        db.get_problems_by_chapter(chapter_id).await.unwrap_or_default()
    } else {
        // Get from same chapter as source
        db.get_problems_by_chapter(&source.chapter_id).await.unwrap_or_default()
    };

    // Find similar
    let detector = SimilarityDetector::new();
    let result = detector.find_similar(&source, &candidates, top_k);

    // Enrich with problem numbers
    let mut similar_problems = Vec::new();
    for sim in result.similar_problems {
        if let Ok(Some(problem)) = db.get_problem(&sim.problem_id).await {
            similar_problems.push(SimilarProblemResponse {
                problem_id: sim.problem_id,
                problem_number: problem.number,
                similarity: sim.similarity,
                match_type: format!("{:?}", sim.match_type),
                reason: sim.reason,
            });
        }
    }

    Ok(HttpResponse::Ok().json(SimilarResponse {
        problem_id: body.problem_id.clone(),
        similar_problems,
    }))
}

// === Recommendations ===

#[derive(Debug, Deserialize)]
pub struct RecommendRequest {
    pub solved_problem_ids: Vec<String>,
    pub chapter_id: String,
    pub count: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct RecommendResponse {
    pub recommendations: Vec<RecommendationResponse>,
}

#[derive(Debug, Serialize)]
pub struct RecommendationResponse {
    pub problem_id: String,
    pub problem_number: String,
    pub reason: String,
    pub similarity: f64,
}

pub async fn recommend_problems(
    body: web::Json<RecommendRequest>,
    db: web::Data<Database>,
) -> Result<HttpResponse, Error> {
    let count = body.count.unwrap_or(5);

    // Get solved problems
    let mut solved = Vec::new();
    for id in &body.solved_problem_ids {
        if let Ok(Some(p)) = db.get_problem(id).await {
            solved.push(p);
        }
    }

    // Get all problems in chapter
    let all_problems = db.get_problems_by_chapter(&body.chapter_id).await.unwrap_or_default();

    // Get recommendations
    let recommender = ProblemRecommender::new();
    let recommendations = recommender.recommend_for_practice(&solved, &all_problems, count);

    // Enrich with problem numbers
    let mut enriched = Vec::new();
    for rec in recommendations {
        if let Ok(Some(problem)) = db.get_problem(&rec.problem_id).await {
            enriched.push(RecommendationResponse {
                problem_id: rec.problem_id,
                problem_number: problem.number,
                reason: rec.reason,
                similarity: rec.similarity,
            });
        }
    }

    Ok(HttpResponse::Ok().json(RecommendResponse {
        recommendations: enriched,
    }))
}

// === Duplicates Detection ===

#[derive(Debug, Deserialize)]
pub struct DuplicatesRequest {
    pub chapter_id: String,
    pub threshold: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct DuplicatesResponse {
    pub groups: Vec<DuplicateGroupResponse>,
}

#[derive(Debug, Serialize)]
pub struct DuplicateGroupResponse {
    pub problem_ids: Vec<String>,
    pub problem_numbers: Vec<String>,
    pub similarity: f64,
}

pub async fn find_duplicates(
    body: web::Json<DuplicatesRequest>,
    db: web::Data<Database>,
) -> Result<HttpResponse, Error> {
    let threshold = body.threshold.unwrap_or(0.85);

    // Get all problems
    let problems = match db.get_problems_by_chapter(&body.chapter_id).await {
        Ok(p) => p,
        Err(e) => {
            return Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Failed to get problems: {}", e)
            })));
        }
    };

    // Find duplicates
    let detector = SimilarityDetector::new();
    let groups = detector.find_duplicates(&problems, threshold);

    // Enrich with problem numbers
    let mut enriched_groups = Vec::new();
    for g in groups {
        let mut numbers = Vec::new();
        for id in &g.problem_ids {
            if let Ok(Some(p)) = db.get_problem(id).await {
                numbers.push(p.number.clone());
            }
        }

        enriched_groups.push(DuplicateGroupResponse {
            problem_ids: g.problem_ids,
            problem_numbers: numbers,
            similarity: g.similarity,
        });
    }

    Ok(HttpResponse::Ok().json(DuplicatesResponse {
        groups: enriched_groups,
    }))
}
