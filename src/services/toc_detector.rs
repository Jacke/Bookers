use regex::Regex;
use crate::models::{Chapter, Book};
use crate::services::database::Database;
use anyhow::Result;

/// Table of Contents detector
pub struct TocDetector;

#[derive(Debug, Clone)]
pub struct TocEntry {
    pub number: u32,
    pub title: String,
    pub page_number: Option<u32>,
    pub level: u8, // 1 = chapter, 2 = section, etc.
}

#[derive(Debug, Clone)]
pub struct DetectedToc {
    pub entries: Vec<TocEntry>,
    pub confidence: f32, // 0.0 - 1.0
}

impl TocDetector {
    pub fn new() -> Self {
        Self
    }

    /// Detect TOC from OCR text
    pub fn detect_toc(&self, text: &str) -> Option<DetectedToc> {
        // Common TOC patterns
        let toc_patterns = vec![
            // "Глава 1. Название главы ............ 15"
            Regex::new(r"(?i)глава\s+(\d+)\.?\s*([^\.]+)\.\.+(\d+)").ok()?,
            // "1. Название главы .................. 15"
            Regex::new(r"(?m)^(\d+)\.\s+([^.]+?)\.\s*\.{3,}\s*(\d+)").ok()?,
            // "§ 1. Название"
            Regex::new(r"§\s*(\d+)\.?\s*([^.\n]+)").ok()?,
            // "Глава I. Название"
            Regex::new(r"(?i)глава\s+([IVXLC]+)\.?\s*([^.\n]+)").ok()?,
        ];

        let mut entries = Vec::new();
        let mut matched_lines = 0;
        let mut total_lines = 0;

        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            total_lines += 1;

            for pattern in &toc_patterns {
                if let Some(caps) = pattern.captures(line) {
                    let num_str = caps.get(1)?.as_str();
                    let title = caps.get(2)?.as_str().trim().to_string();
                    
                    // Parse number ( Arabic or Roman )
                    let number = if let Ok(n) = num_str.parse::<u32>() {
                        n
                    } else {
                        Self::roman_to_arabic(num_str).unwrap_or(0)
                    };

                    // Extract page number if present
                    let page_number = caps.get(3)
                        .and_then(|m| m.as_str().trim().parse::<u32>().ok());

                    entries.push(TocEntry {
                        number,
                        title,
                        page_number,
                        level: 1,
                    });

                    matched_lines += 1;
                    break;
                }
            }
        }

        if entries.is_empty() {
            return None;
        }

        // Calculate confidence based on matched lines ratio
        let confidence = if total_lines > 0 {
            (matched_lines as f32 / total_lines as f32).min(1.0)
        } else {
            0.0
        };

        Some(DetectedToc { entries, confidence })
    }

    /// Detect chapters from page-by-page OCR text
    pub fn detect_chapters_from_pages(&self, pages: &[(u32, String)]) -> Vec<ChapterDetection> {
        let mut chapters = Vec::new();
        let mut last_number = 0;

        // Pattern for chapter headers on pages
        let chapter_patterns = vec![
            Regex::new(r"(?i)^\s*глава\s+(\d+)").ok(),
            Regex::new(r"(?i)^\s*глава\s+([IVXLC]+)").ok(),
            Regex::new(r"(?i)^\s*§\s*(\d+)").ok(),
            Regex::new(r"(?i)^\s*раздел\s+(\d+)").ok(),
            Regex::new(r"(?i)^\s*chapter\s+(\d+)").ok(),
        ];

        for (page_num, text) in pages {
            let first_lines: String = text.lines().take(10).collect::<Vec<_>>().join("\n");

            for pattern_opt in &chapter_patterns {
                if let Some(pattern) = pattern_opt {
                    if let Some(caps) = pattern.captures(&first_lines) {
                        let num_str = caps.get(1).map(|m| m.as_str()).unwrap_or("0");
                        
                        let number = if let Ok(n) = num_str.parse::<u32>() {
                            n
                        } else {
                            Self::roman_to_arabic(num_str).unwrap_or(0)
                        };

                        // Only accept if number is sequential
                        if number > last_number || number == 1 {
                            // Extract title from next lines
                            let title = self.extract_chapter_title(text, caps.get(0).unwrap().end());

                            chapters.push(ChapterDetection {
                                number,
                                title,
                                start_page: *page_num,
                                end_page: None,
                                confidence: 0.9,
                            });
                            last_number = number;
                        }
                        break;
                    }
                }
            }
        }

        // Calculate end pages
        for i in 0..chapters.len() {
            if i + 1 < chapters.len() {
                chapters[i].end_page = Some(chapters[i + 1].start_page - 1);
            }
        }

        chapters
    }

    /// Extract chapter title from text after header
    fn extract_chapter_title(&self, text: &str, start_pos: usize) -> String {
        let remaining = &text[start_pos..];
        
        // Take first non-empty line as title
        for line in remaining.lines() {
            let line = line.trim();
            if !line.is_empty() && line.len() < 200 {
                // Clean up the title
                return line
                    .trim_start_matches('.')
                    .trim_start_matches(':')
                    .trim()
                    .to_string();
            }
        }

        "Untitled Chapter".to_string()
    }

    /// Convert Roman numerals to Arabic
    fn roman_to_arabic(roman: &str) -> Option<u32> {
        let roman = roman.to_uppercase();
        let mut result = 0;
        let mut prev_value = 0;

        for c in roman.chars().rev() {
            let value = match c {
                'I' => 1,
                'V' => 5,
                'X' => 10,
                'L' => 50,
                'C' => 100,
                'D' => 500,
                'M' => 1000,
                _ => return None,
            };

            if value < prev_value {
                result -= value;
            } else {
                result += value;
            }
            prev_value = value;
        }

        Some(result)
    }

    /// Smart chapter creation from TOC detection
    pub async fn create_chapters_from_toc(
        &self,
        db: &Database,
        book: &Book,
        toc: &DetectedToc,
    ) -> Result<Vec<Chapter>> {
        let mut chapters = Vec::new();

        for (i, entry) in toc.entries.iter().enumerate() {
            let chapter_id = format!("{}:{}", book.id, entry.number);
            
            // Calculate end page
            let end_page = if i + 1 < toc.entries.len() {
                toc.entries[i + 1].page_number.map(|p| p - 1)
            } else {
                Some(book.total_pages)
            };

            let chapter = Chapter {
                id: chapter_id,
                book_id: book.id.clone(),
                number: entry.number,
                title: entry.title.clone(),
                description: entry.page_number.map(|p| {
                    format!("Pages {}-{}", p, end_page.unwrap_or(book.total_pages))
                }),
                problem_count: 0,
                theory_count: 0,
                created_at: chrono::Utc::now(),
            };

            // Save to database
            db.create_chapter(&chapter).await?;
            chapters.push(chapter);
        }

        Ok(chapters)
    }
}

#[derive(Debug, Clone)]
pub struct ChapterDetection {
    pub number: u32,
    pub title: String,
    pub start_page: u32,
    pub end_page: Option<u32>,
    pub confidence: f32,
}

/// Batch import with auto chapter detection
pub struct SmartImporter {
    toc_detector: TocDetector,
}

impl SmartImporter {
    pub fn new() -> Self {
        Self {
            toc_detector: TocDetector::new(),
        }
    }

    /// Import book with automatic chapter detection
    pub async fn import_book_with_chapters(
        &self,
        db: &Database,
        book_id: &str,
        title: &str,
        total_pages: u32,
        toc_page_ocr: Option<&str>,
    ) -> Result<ImportResult> {
        // Create or update book
        let book = Book {
            id: book_id.to_string(),
            title: title.to_string(),
            author: None,
            subject: None,
            file_path: format!("resources/{}.pdf", book_id),
            total_pages,
            created_at: chrono::Utc::now(),
        };

        db.create_book(&book).await?;

        let mut chapters = Vec::new();
        let mut detection_source = "none";

        // Try to detect TOC from provided OCR
        if let Some(toc_text) = toc_page_ocr {
            if let Some(toc) = self.toc_detector.detect_toc(toc_text) {
                if toc.confidence > 0.5 {
                    chapters = self.toc_detector.create_chapters_from_toc(db, &book, &toc).await?;
                    detection_source = "toc_page";
                }
            }
        }

        // If no chapters detected, create default chapter
        if chapters.is_empty() {
            let default_chapter = Chapter {
                id: format!("{}:1", book_id),
                book_id: book_id.to_string(),
                number: 1,
                title: "Глава 1".to_string(),
                description: Some(format!("Pages 1-{}", total_pages)),
                problem_count: 0,
                theory_count: 0,
                created_at: chrono::Utc::now(),
            };

            db.create_chapter(&default_chapter).await?;
            chapters.push(default_chapter);
            detection_source = "default";
        }

        Ok(ImportResult {
            book,
            chapters,
            detection_source: detection_source.to_string(),
        })
    }
}

pub struct ImportResult {
    pub book: Book,
    pub chapters: Vec<Chapter>,
    pub detection_source: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roman_to_arabic() {
        assert_eq!(TocDetector::roman_to_arabic("I"), Some(1));
        assert_eq!(TocDetector::roman_to_arabic("V"), Some(5));
        assert_eq!(TocDetector::roman_to_arabic("X"), Some(10));
        assert_eq!(TocDetector::roman_to_arabic("XIV"), Some(14));
        assert_eq!(TocDetector::roman_to_arabic("XLII"), Some(42));
    }

    #[test]
    fn test_detect_toc() {
        let detector = TocDetector::new();
        
        let toc_text = r#"
Оглавление

Глава 1. Введение в алгебру .................. 5
Глава 2. Линейные уравнения .................. 25
Глава 3. Квадратные уравнения ................ 47

Приложение ................................... 120
        "#;

        let result = detector.detect_toc(toc_text);
        assert!(result.is_some());
        
        let toc = result.unwrap();
        assert_eq!(toc.entries.len(), 3);
        assert_eq!(toc.entries[0].number, 1);
        assert_eq!(toc.entries[0].title, "Введение в алгебру");
        assert_eq!(toc.entries[0].page_number, Some(5));
    }
}
