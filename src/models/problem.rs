use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Unique problem ID format: {book_id}:{chapter_num}:{problem_num}
/// Example: "algebra-7:3:15"
pub type ProblemId = String;
pub type TheoryId = String;
pub type SolutionId = String;

/// Represents a math problem extracted from textbook
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Problem {
    pub id: ProblemId,
    pub chapter_id: String,
    /// Parent page ID (if created from OCR page)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_id: Option<String>,
    /// Parent problem ID (for sub-problems like а, б, в...)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    /// Problem number as shown in textbook (e.g., "1.1", "125", "5а")
    pub number: String,
    /// Display ID for UI (e.g., "Задача 1", "Problem 5")
    pub display_name: String,
    /// Problem content in Markdown with LaTeX
    pub content: String,
    /// Extracted LaTeX formulas for indexing/search
    pub latex_formulas: Vec<String>,
    /// Page number in PDF
    pub page_number: Option<u32>,
    /// Estimated difficulty (1-10, optional)
    pub difficulty: Option<u8>,
    /// Has verified solution
    pub has_solution: bool,
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
    /// Linked solution (loaded on demand)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub solution: Option<Solution>,
    /// Sub-problems (loaded on demand)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub_problems: Option<Vec<Problem>>,
    /// Cross-page: continues from previous page
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continues_from_page: Option<u32>,
    /// Cross-page: continues to next page
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continues_to_page: Option<u32>,
    /// Is this problem split across multiple pages?
    #[serde(default)]
    pub is_cross_page: bool,
}

/// Represents a PDF page with OCR text
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page {
    pub id: String,
    pub book_id: String,
    pub page_number: u32,
    pub ocr_text: Option<String>,
    pub has_problems: bool,
    pub problem_count: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Represents a theory/explanation block from textbook
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TheoryBlock {
    pub id: TheoryId,
    pub chapter_id: String,
    /// Block sequence number within chapter (T-1, T-2, ...)
    pub block_num: u32,
    /// Optional title (e.g., "Теорема Пифагора")
    pub title: Option<String>,
    /// Block type
    pub block_type: TheoryType,
    /// Content in Markdown with LaTeX
    pub content: String,
    /// Extracted LaTeX formulas
    pub latex_formulas: Vec<String>,
    /// Page number in PDF
    pub page_number: Option<u32>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TheoryType {
    Definition,    // Определение
    Theorem,       // Теорема
    Proof,         // Доказательство
    Property,      // Свойство
    Formula,       // Формула
    Explanation,   // Пояснение
    Example,       // Пример
    Other,
}

/// AI-generated solution for a problem
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Solution {
    pub id: SolutionId,
    pub problem_id: ProblemId,
    /// AI provider used (openai, claude, mistral, etc.)
    pub provider: String,
    /// Solution content in Markdown with LaTeX
    pub content: String,
    /// Extracted LaTeX formulas
    pub latex_formulas: Vec<String>,
    /// Whether user verified this solution is correct
    pub is_verified: bool,
    /// User rating (1-5)
    pub rating: Option<u8>,
    /// Generation timestamp
    pub created_at: DateTime<Utc>,
    /// Last update timestamp
    pub updated_at: DateTime<Utc>,
}

/// Chapter/section of a book
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chapter {
    pub id: String,
    pub book_id: String,
    /// Chapter number (1, 2, 3...)
    pub number: u32,
    pub title: String,
    /// Brief description
    pub description: Option<String>,
    /// Number of problems in chapter
    pub problem_count: u32,
    /// Number of theory blocks
    pub theory_count: u32,
    pub created_at: DateTime<Utc>,
}

/// Book/Textbook metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Book {
    pub id: String,
    pub title: String,
    pub author: Option<String>,
    pub subject: Option<String>, // algebra, geometry, calculus, etc.
    pub file_path: String,
    pub total_pages: u32,
    pub created_at: DateTime<Utc>,
}

/// Request to generate solution
#[derive(Debug, Deserialize)]
pub struct SolveRequest {
    pub provider: Option<String>, // openai, claude, mistral
    pub force_regenerate: Option<bool>,
    pub custom_prompt: Option<String>,
}

/// Response with solution
#[derive(Debug, Serialize)]
pub struct SolutionResponse {
    pub problem: Problem,
    pub solution: Solution,
    pub generation_time_ms: u64,
}

/// Problem with truncated info (for lists)
#[derive(Debug, Serialize)]
pub struct ProblemSummary {
    pub id: ProblemId,
    pub number: String,
    pub display_name: String,
    pub preview: String, // First 100 chars
    pub has_solution: bool,
    pub difficulty: Option<u8>,
}

/// Search result for formula search
#[derive(Debug, Serialize)]
pub struct FormulaSearchResult {
    pub problems: Vec<Problem>,
    pub theory_blocks: Vec<TheoryBlock>,
}

impl Problem {
    /// Generate unique problem ID
    pub fn generate_id(book_id: &str, chapter_num: u32, problem_num: &str) -> ProblemId {
        format!("{}:{}:{}", book_id, chapter_num, problem_num)
    }

    /// Extract LaTeX formulas from content for indexing
    pub fn extract_formulas(&self) -> Vec<String> {
        let mut formulas = Vec::new();
        // Pattern for inline math: $...$
        let inline_re = regex::Regex::new(r"\$([^$]+)\$").unwrap();
        // Pattern for display math: $$...$$ or \[...\]
        let display_re = regex::Regex::new(r"\$\$([^$]+)\$\$|\\\[([^\]]+)\\\]").unwrap();

        for cap in inline_re.captures_iter(&self.content) {
            formulas.push(cap[1].to_string());
        }
        for cap in display_re.captures_iter(&self.content) {
            if let Some(m) = cap.get(1) {
                formulas.push(m.as_str().to_string());
            } else if let Some(m) = cap.get(2) {
                formulas.push(m.as_str().to_string());
            }
        }

        formulas
    }
}

impl TheoryBlock {
    /// Generate unique theory ID
    pub fn generate_id(book_id: &str, chapter_num: u32, block_num: u32) -> TheoryId {
        format!("{}:{}:T:{}", book_id, chapter_num, block_num)
    }
}

impl Solution {
    /// Generate unique solution ID
    pub fn generate_id(problem_id: &ProblemId) -> SolutionId {
        format!("{}:S:{}", problem_id, uuid::Uuid::new_v4())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_problem_id_generation() {
        let id = Problem::generate_id("algebra-7", 3, "15");
        assert_eq!(id, "algebra-7:3:15");
    }

    #[test]
    fn test_formula_extraction() {
        let problem = Problem {
            id: "test".to_string(),
            chapter_id: "test".to_string(),
            number: "1".to_string(),
            display_name: "Test".to_string(),
            content: "Solve $x^2 + y^2 = z^2$ and $$\\int_0^1 x dx$$".to_string(),
            latex_formulas: vec![],
            page_number: None,
            difficulty: None,
            has_solution: false,
            created_at: Utc::now(),
            solution: None,
        };

        let formulas = problem.extract_formulas();
        assert!(formulas.contains(&"x^2 + y^2 = z^2".to_string()));
    }
}
