use std::sync::Arc;
use crate::config::Config;
use crate::services::background::{JobManager, JobType, JobStatus};
use crate::services::database::Database;
use crate::services::ai_parser::HybridParser;
use crate::services::ocr::OcrService;

/// Batch OCR processor
pub struct BatchProcessor {
    job_manager: Arc<JobManager>,
    db: Arc<Database>,
    config: Arc<Config>,
}

#[derive(Debug, Clone)]
pub struct BatchOcrResult {
    pub processed_pages: u32,
    pub problems_found: u32,
    pub errors: Vec<String>,
    pub duration_secs: u64,
}

#[derive(Debug, Clone)]
pub struct BatchSolveResult {
    pub processed: u32,
    pub succeeded: u32,
    pub failed: u32,
    pub duration_secs: u64,
}

impl BatchProcessor {
    pub fn new(
        job_manager: Arc<JobManager>,
        db: Arc<Database>,
        config: Arc<Config>,
    ) -> Self {
        Self { job_manager, db, config }
    }
    
    /// Start batch OCR job
    pub async fn start_batch_ocr(&self, book_id: &str, start_page: u32, end_page: u32, chapter_id: &str) -> anyhow::Result<String> {
        let job_id = self.job_manager.create_job(JobType::BatchOcr {
            book_id: book_id.to_string(),
            page_range: (start_page, end_page),
            chapter_id: chapter_id.to_string(),
        }).await;
        
        let processor = self.clone();
        let jid = job_id.clone();
        let book_id = book_id.to_string();
        let chapter_id = chapter_id.to_string();
        
        tokio::spawn(async move {
            processor.run_batch_ocr(&jid, &book_id, start_page, end_page, &chapter_id).await;
        });
        
        Ok(job_id)
    }
    
    async fn run_batch_ocr(&self, job_id: &str, book_id: &str, start_page: u32, end_page: u32, chapter_id: &str) {
        let start_time = std::time::Instant::now();
        let total_pages = end_page - start_page + 1;
        let mut processed = 0u32;
        let mut total_problems = 0u32;
        let mut errors = Vec::new();
        // Tail text detected at page boundary (e.g. "Глава 5...") that should be
        // parsed with the next page, not appended to the last problem of current page.
        let mut carryover_text = String::new();
        
        // Get book info
        let _book = match self.db.get_book(book_id).await {
            Ok(Some(b)) => b,
            _ => {
                self.job_manager.fail_job(job_id, &format!("Book not found: {}", book_id)).await;
                return;
            }
        };
        
        let parser = HybridParser::new(std::env::var("MISTRAL_API_KEY").ok());
        let ocr_service = OcrService::new(self.config.preview_dir.clone());
        
        for page_num in start_page..=end_page {
            // Check if job was cancelled
            if let Some(job) = self.job_manager.get_job(job_id).await {
                if matches!(job.status, JobStatus::Cancelled) {
                    return;
                }
            }
            
            let progress = processed as f32 / total_pages as f32 * 100.0;
            self.job_manager.update_progress(
                job_id,
                progress,
                &format!("Processing page {} of {}", page_num, end_page)
            ).await;
            
            // Run OCR on page
            let filename = format!("{}.pdf", book_id);
            let image_path = self.config.preview_dir.join(format!("{}_{}.png", filename, page_num));
            
            let ocr_text = match ocr_service.run_ocr(&image_path, "mistral").await {
                Ok(text) => text,
                Err(e) => {
                    errors.push(format!("Page {}: OCR failed - {}", page_num, e));
                    processed += 1;
                    continue;
                }
            };

            let merged_text = if carryover_text.is_empty() {
                ocr_text
            } else {
                format!("{}\n\n{}", carryover_text, ocr_text)
            };
            let (page_text, next_carryover) = split_trailing_chapter_heading(&merged_text);
            carryover_text = next_carryover.unwrap_or_default();
            
            // Parse problems
            let parse_result = match parser.parse_text(book_id, &page_text, Some(page_num)).await {
                Ok(r) => r,
                Err(e) => {
                    errors.push(format!("Page {}: Parse failed - {}", page_num, e));
                    processed += 1;
                    continue;
                }
            };
            
            // Get or create page
            let page = match self.db.get_or_create_page(book_id, page_num).await {
                Ok(p) => p,
                Err(e) => {
                    errors.push(format!("Page {}: Failed to create page - {}", page_num, e));
                    processed += 1;
                    continue;
                }
            };
            
            // Delete old problems on this page
            let _ = self.db.delete_problems_by_page(&page.id).await;
            
            // Update page OCR
            let _ = self
                .db
                .update_page_ocr(&page.id, &page_text, parse_result.problems.len() as u32)
                .await;
            
            // Create problems
            let chapter_num: u32 = chapter_id.split(':').last()
                .and_then(|s| s.parse().ok())
                .unwrap_or(1);
            
            let mut problems_to_create = Vec::new();
            for ai_problem in &parse_result.problems {
                let problem_id = format!("{}:{}:{}", book_id, chapter_num, ai_problem.number);
                
                let main_problem = crate::models::Problem {
                    id: problem_id.clone(),
                    chapter_id: chapter_id.to_string(),
                    page_id: Some(page.id.clone()),
                    parent_id: None,
                    number: ai_problem.number.clone(),
                    display_name: format!("Задача {}", ai_problem.number),
                    content: ai_problem.content.clone(),
                    latex_formulas: extract_formulas(&ai_problem.content),
                    page_number: Some(page_num),
                    difficulty: None,
                    has_solution: false,
                    created_at: chrono::Utc::now(),
                    solution: None,
                    sub_problems: None,
                    continues_from_page: if ai_problem.continues_from_prev { 
                        Some(page_num.saturating_sub(1)) 
                    } else { None },
                    continues_to_page: if ai_problem.continues_to_next { 
                        Some(page_num + 1) 
                    } else { None },
                    is_cross_page: ai_problem.continues_from_prev || ai_problem.continues_to_next,
                };
                
                problems_to_create.push(main_problem);
                total_problems += 1;
                
                // Create sub-problems
                for sub in &ai_problem.sub_problems {
                    let sub_id = format!("{}:{}", problem_id, sub.letter);
                    let sub_problem = crate::models::Problem {
                        id: sub_id,
                        chapter_id: chapter_id.to_string(),
                        page_id: Some(page.id.clone()),
                        parent_id: Some(problem_id.clone()),
                        number: sub.letter.clone(),
                        display_name: format!("{})", sub.letter),
                        content: sub.content.clone(),
                        latex_formulas: extract_formulas(&sub.content),
                        page_number: Some(page_num),
                        difficulty: None,
                        has_solution: false,
                        created_at: chrono::Utc::now(),
                        solution: None,
                        sub_problems: None,
                        continues_from_page: None,
                        continues_to_page: None,
                        is_cross_page: false,
                    };
                    problems_to_create.push(sub_problem);
                }
            }
            
            // Save to database
            if let Err(e) = self.db.create_or_update_problems(&problems_to_create).await {
                errors.push(format!("Page {}: Failed to save problems - {}", page_num, e));
            }
            
            processed += 1;
            
            // Small delay to avoid rate limiting
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }

        if !carryover_text.trim().is_empty() {
            log::info!(
                "Batch OCR ended with unconsumed carryover text: {} chars",
                carryover_text.len()
            );
        }
        
        let duration = start_time.elapsed().as_secs();
        
        let result = serde_json::json!({
            "processed_pages": processed,
            "problems_found": total_problems,
            "errors": errors,
            "duration_secs": duration,
        });
        
        self.job_manager.complete_job(job_id, result).await;
    }
    
    /// Start batch solve job
    pub async fn start_batch_solve(&self, problem_ids: Vec<String>, provider: &str) -> anyhow::Result<String> {
        let job_id = self.job_manager.create_job(JobType::BatchSolve {
            problem_ids: problem_ids.clone(),
            provider: provider.to_string(),
        }).await;
        
        let processor = self.clone();
        let jid = job_id.clone();
        let prov = provider.to_string();
        
        tokio::spawn(async move {
            processor.run_batch_solve(&jid, problem_ids, &prov).await;
        });
        
        Ok(job_id)
    }
    
    async fn run_batch_solve(&self, job_id: &str, problem_ids: Vec<String>, provider: &str) {
        use crate::services::ai_solver::AISolver;
        
        let start_time = std::time::Instant::now();
        let total = problem_ids.len() as u32;
        let mut processed = 0u32;
        let mut succeeded = 0u32;
        let mut failed = 0u32;
        
        let solver = AISolver::new(&self.config).expect("Failed to create AI solver");
        
        for problem_id in problem_ids {
            // Check if job was cancelled
            if let Some(job) = self.job_manager.get_job(job_id).await {
                if matches!(job.status, JobStatus::Cancelled) {
                    return;
                }
            }
            
            let progress = processed as f32 / total as f32 * 100.0;
            self.job_manager.update_progress(
                job_id,
                progress,
                &format!("Solving problem {}", problem_id)
            ).await;
            
            // Get problem
            let problem = match self.db.get_problem(&problem_id).await {
                Ok(Some(p)) => p,
                _ => {
                    failed += 1;
                    processed += 1;
                    continue;
                }
            };
            
            // Skip if already has solution and not force regenerate
            if problem.has_solution {
                succeeded += 1;
                processed += 1;
                continue;
            }
            
            // Generate solution
            match solver.solve(&problem, Some(provider), None).await {
                Ok(solution) => {
                    // Save solution
                    if let Err(e) = self.db.save_solution(&solution).await {
                        log::error!("Failed to save solution: {}", e);
                        failed += 1;
                    } else {
                        // Update problem status
                        let _ = self.db.update_problem_solution_status(&problem_id, true).await;
                        succeeded += 1;
                    }
                }
                Err(e) => {
                    log::error!("Failed to generate solution: {}", e);
                    failed += 1;
                }
            }
            
            processed += 1;
            
            // Delay to avoid rate limiting
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        }
        
        let duration = start_time.elapsed().as_secs();
        
        let result = serde_json::json!({
            "processed": processed,
            "succeeded": succeeded,
            "failed": failed,
            "duration_secs": duration,
        });
        
        self.job_manager.complete_job(job_id, result).await;
    }
}

impl Clone for BatchProcessor {
    fn clone(&self) -> Self {
        Self {
            job_manager: self.job_manager.clone(),
            db: self.db.clone(),
            config: self.config.clone(),
        }
    }
}

fn extract_formulas(text: &str) -> Vec<String> {
    let mut formulas = Vec::new();
    let re = regex::Regex::new(r"\$([^$]+)\$").unwrap();
    for cap in re.captures_iter(text) {
        formulas.push(cap[1].to_string());
    }
    formulas
}

fn split_trailing_chapter_heading(text: &str) -> (String, Option<String>) {
    let lines: Vec<&str> = text.lines().collect();
    let Some(last_non_empty_idx) = lines.iter().rposition(|l| !l.trim().is_empty()) else {
        return (String::new(), None);
    };

    if !is_chapter_heading_line(lines[last_non_empty_idx]) {
        return (text.trim().to_string(), None);
    }

    let current = lines[..last_non_empty_idx].join("\n").trim().to_string();
    let carryover = lines[last_non_empty_idx..].join("\n").trim().to_string();

    if carryover.is_empty() {
        (current, None)
    } else {
        (current, Some(carryover))
    }
}

fn is_chapter_heading_line(line: &str) -> bool {
    let lower = line.trim().to_lowercase();
    let Some(rest) = lower.strip_prefix("глава") else {
        return false;
    };

    let rest = rest.trim_start();
    if rest.is_empty() {
        return false;
    }

    let chapter_token: String = rest
        .chars()
        .take_while(|c| !c.is_whitespace() && *c != '.' && *c != ':')
        .collect();

    if chapter_token.is_empty() {
        return false;
    }

    let is_digits = chapter_token.chars().all(|c| c.is_ascii_digit());
    let is_roman = chapter_token
        .chars()
        .all(|c| matches!(c, 'i' | 'v' | 'x' | 'l' | 'c' | 'd' | 'm'));

    is_digits || is_roman
}

#[cfg(test)]
mod tests {
    use super::split_trailing_chapter_heading;

    #[test]
    fn splits_trailing_chapter_header_into_carryover() {
        let text = "702. Последняя задача.\nГлава 5. Разложение многочленов на множители";
        let (page_text, carryover) = split_trailing_chapter_heading(text);

        assert_eq!(page_text, "702. Последняя задача.");
        assert_eq!(
            carryover.as_deref(),
            Some("Глава 5. Разложение многочленов на множители")
        );
    }

    #[test]
    fn leaves_text_intact_without_trailing_chapter_header() {
        let text = "701. Обычная задача без заголовка главы.";
        let (page_text, carryover) = split_trailing_chapter_heading(text);

        assert_eq!(page_text, text);
        assert!(carryover.is_none());
    }
}
