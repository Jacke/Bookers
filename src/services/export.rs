use crate::models::{Book, Chapter, Problem};
use crate::services::database::Database;
use anyhow::Result;

/// Export formats
#[derive(Debug, Clone, Copy)]
pub enum ExportFormat {
    Markdown,
    Latex,
    Json,
    Anki,
}

impl ExportFormat {
    pub fn extension(&self) -> &'static str {
        match self {
            ExportFormat::Markdown => "md",
            ExportFormat::Latex => "tex",
            ExportFormat::Json => "json",
            ExportFormat::Anki => "apkg",
        }
    }
    
    pub fn mime_type(&self) -> &'static str {
        match self {
            ExportFormat::Markdown => "text/markdown",
            ExportFormat::Latex => "application/x-latex",
            ExportFormat::Json => "application/json",
            ExportFormat::Anki => "application/octet-stream",
        }
    }
}

/// Exporter service
pub struct Exporter {
    db: Database,
}

impl Exporter {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
    
    /// Export entire book
    pub async fn export_book(&self, book_id: &str, format: ExportFormat) -> Result<Vec<u8>> {
        let book = self.db.get_book(book_id).await?
            .ok_or_else(|| anyhow::anyhow!("Book not found"))?;
        
        match format {
            ExportFormat::Markdown => self.export_markdown(&book).await,
            ExportFormat::Latex => self.export_latex(&book).await,
            ExportFormat::Json => self.export_json(&book).await,
            ExportFormat::Anki => self.export_anki(&book).await,
        }
    }
    
    /// Export single chapter
    pub async fn export_chapter(&self, chapter_id: &str, format: ExportFormat) -> Result<Vec<u8>> {
        let chapter = self.db.get_chapter(chapter_id).await?
            .ok_or_else(|| anyhow::anyhow!("Chapter not found"))?;
        
        let book = self.db.get_book(&chapter.book_id).await?
            .ok_or_else(|| anyhow::anyhow!("Book not found"))?;
        
        match format {
            ExportFormat::Markdown => self.export_chapter_markdown(&book, &chapter).await,
            ExportFormat::Latex => self.export_chapter_latex(&book, &chapter).await,
            ExportFormat::Json => self.export_chapter_json(&book, &chapter).await,
            ExportFormat::Anki => self.export_chapter_anki(&book, &chapter).await,
        }
    }
    
    async fn export_markdown(&self, book: &Book) -> Result<Vec<u8>> {
        let mut output = String::new();
        
        // Title
        output.push_str(&format!("# {}\n\n", book.title));
        
        if let Some(author) = &book.author {
            output.push_str(&format!("**Автор:** {}\n\n", author));
        }
        
        // Get all chapters
        let chapters = self.db.get_chapters_by_book(&book.id).await?;
        
        for chapter in chapters {
            output.push_str(&self.export_chapter_markdown_content(&chapter).await?);
        }
        
        Ok(output.into_bytes())
    }
    
    async fn export_chapter_markdown(&self, book: &Book, chapter: &Chapter) -> Result<Vec<u8>> {
        let mut output = String::new();
        
        output.push_str(&format!("# {}\n\n", book.title));
        output.push_str(&format!("## Глава {}: {}\n\n", chapter.number, chapter.title));
        
        output.push_str(&self.export_chapter_markdown_content(chapter).await?);
        
        Ok(output.into_bytes())
    }
    
    async fn export_chapter_markdown_content(&self, chapter: &Chapter) -> Result<String> {
        let mut output = String::new();
        
        output.push_str(&format!("### Глава {}: {}\n\n", chapter.number, chapter.title));
        
        // Get problems
        let problems = self.db.get_problems_by_chapter(&chapter.id).await?;
        
        for problem in problems {
            // Skip sub-problems (they'll be included with parent)
            if problem.parent_id.is_some() {
                continue;
            }
            
            output.push_str(&self.format_problem_markdown(&problem).await?);
        }
        
        Ok(output)
    }
    
    async fn format_problem_markdown(&self, problem: &Problem) -> Result<String> {
        let mut output = String::new();
        
        // Problem header
        output.push_str(&format!("#### Задача {}\n\n", problem.number));
        
        // Content with preserved LaTeX
        output.push_str(&problem.content);
        output.push_str("\n\n");
        
        // Sub-problems
        if let Some(subs) = &problem.sub_problems {
            for sub in subs {
                output.push_str(&format!("**{}).** {}\n\n", sub.number, sub.content));
            }
        }
        
        // Solution if exists
        if problem.has_solution {
            if let Some(solution) = self.db.get_solution_for_problem(&problem.id).await? {
                output.push_str("**Решение:**\n\n");
                output.push_str(&solution.content);
                output.push_str("\n\n");
            }
        }
        
        output.push_str("---\n\n");
        
        Ok(output)
    }
    
    async fn export_latex(&self, book: &Book) -> Result<Vec<u8>> {
        let mut output = String::new();
        
        // LaTeX preamble
        output.push_str(r"\documentclass{article}
\usepackage[utf8]{inputenc}
\usepackage[russian]{babel}
\usepackage{amsmath,amssymb,amsthm}
\usepackage{geometry}
\geometry{a4paper,margin=2cm}

\title{");
        output.push_str(&book.title);
        output.push_str(r"}
\date{\today}

\begin{document}
\maketitle

");
        
        // Chapters
        let chapters = self.db.get_chapters_by_book(&book.id).await?;
        
        for chapter in chapters {
            output.push_str(&format!("\\section*{{Глава {}: {}}}\n\n", chapter.number, chapter.title));
            
            let problems = self.db.get_problems_by_chapter(&chapter.id).await?;
            
            for problem in problems {
                if problem.parent_id.is_some() {
                    continue;
                }
                
                output.push_str(&self.format_problem_latex(&problem).await?);
            }
        }
        
        output.push_str(r"\end{document}");
        
        Ok(output.into_bytes())
    }
    
    async fn format_problem_latex(&self, problem: &Problem) -> Result<String> {
        let mut output = String::new();
        
        output.push_str(&format!("\\textbf{{Задача {}.}} ", problem.number));
        
        // Convert markdown LaTeX to LaTeX
        let content = problem.content
            .replace("$", "$")  // Keep inline math
            .replace("$$", r"\[",)  // Display math opening
            .replace("$$", r"\]",); // Display math closing
        
        output.push_str(&content);
        output.push_str("\n\n");
        
        // Sub-problems
        if let Some(subs) = &problem.sub_problems {
            output.push_str(r"\begin{enumerate}[label=\alph*)]");
            for sub in subs {
                output.push_str(&format!("\\item {}\n", sub.content));
            }
            output.push_str(r"\end{enumerate}");
            output.push_str("\n\n");
        }
        
        Ok(output)
    }
    
    async fn export_json(&self, book: &Book) -> Result<Vec<u8>> {
        let chapters = self.db.get_chapters_by_book(&book.id).await?;
        
        let mut export_data = serde_json::Map::new();
        export_data.insert("book".to_string(), serde_json::json!({
            "id": book.id,
            "title": book.title,
            "author": book.author,
            "subject": book.subject,
        }));
        
        let mut chapters_data = Vec::new();
        
        for chapter in chapters {
            let problems = self.db.get_problems_by_chapter(&chapter.id).await?;
            
            chapters_data.push(serde_json::json!({
                "id": chapter.id,
                "number": chapter.number,
                "title": chapter.title,
                "problems": problems.iter().filter(|p| p.parent_id.is_none()).map(|p| {
                    serde_json::json!({
                        "id": p.id,
                        "number": p.number,
                        "content": p.content,
                        "latex_formulas": p.latex_formulas,
                        "sub_problems": p.sub_problems,
                        "has_solution": p.has_solution,
                    })
                }).collect::<Vec<_>>(),
            }));
        }
        
        export_data.insert("chapters".to_string(), serde_json::Value::Array(chapters_data));
        
        let json = serde_json::to_string_pretty(&export_data)?;
        Ok(json.into_bytes())
    }
    
    async fn export_anki(&self, book: &Book) -> Result<Vec<u8>> {
        // For Anki, we generate a CSV-like format that can be imported
        // Real .apkg generation would require additional dependencies
        
        let mut output = String::new();
        
        // Header
        output.push_str("#separator:tab\n");
        output.push_str("#html:true\n");
        output.push_str("#deck column:1\n");
        output.push_str("#tags column:4\n\n");
        
        let chapters = self.db.get_chapters_by_book(&book.id).await?;
        
        for chapter in chapters {
            let problems = self.db.get_problems_by_chapter(&chapter.id).await?;
            
            for problem in problems {
                if problem.parent_id.is_some() {
                    continue;
                }
                
                // Front (question)
                let front = format!("{} - Задача {}", book.title, problem.number);
                let front_html = format!("<b>{}</b><br><br>{}", 
                    front, 
                    problem.content.replace("$", "&#36;")
                );
                
                // Back (solution or hint)
                let back_html = if let Some(solution) = self.db.get_solution_for_problem(&problem.id).await? {
                    solution.content.replace("$", "&#36;")
                } else {
                    "(Решение не добавлено)".to_string()
                };
                
                // Tags
                let tags = format!("{}::chapter_{}", book.id.replace("-", "_"), chapter.number);
                
                output.push_str(&format!("{}\t{}\t{}\t{}\n", 
                    format!("{}::Глава {}", book.title, chapter.number),
                    front_html,
                    back_html,
                    tags
                ));
            }
        }
        
        Ok(output.into_bytes())
    }
    
    // Chapter-specific exports
    async fn export_chapter_latex(&self, book: &Book, chapter: &Chapter) -> Result<Vec<u8>> {
        let mut output = String::new();
        
        output.push_str(r"\documentclass{article}
\usepackage[utf8]{inputenc}
\usepackage[russian]{babel}
\usepackage{amsmath,amssymb,amsthm}
\usepackage{geometry}
\geometry{a4paper,margin=2cm}

\title{");
        output.push_str(&format!("{} - Глава {}", book.title, chapter.number));
        output.push_str(r"}
\author{");
        if let Some(author) = &book.author {
            output.push_str(author);
        }
        output.push_str(r"}
\date{\today}

\begin{document}
\maketitle

");
        
        output.push_str(&format!("\\section*{{{}}}\n\n", chapter.title));
        
        let problems = self.db.get_problems_by_chapter(&chapter.id).await?;
        
        for problem in problems {
            if problem.parent_id.is_some() {
                continue;
            }
            output.push_str(&self.format_problem_latex(&problem).await?);
        }
        
        output.push_str(r"\end{document}");
        
        Ok(output.into_bytes())
    }
    
    async fn export_chapter_json(&self, _book: &Book, chapter: &Chapter) -> Result<Vec<u8>> {
        let problems = self.db.get_problems_by_chapter(&chapter.id).await?;
        
        let export_data = serde_json::json!({
            "chapter": {
                "id": chapter.id,
                "number": chapter.number,
                "title": chapter.title,
            },
            "problems": problems.iter().filter(|p| p.parent_id.is_none()).map(|p| {
                serde_json::json!({
                    "id": p.id,
                    "number": p.number,
                    "content": p.content,
                    "latex_formulas": p.latex_formulas,
                    "sub_problems": p.sub_problems,
                    "page_number": p.page_number,
                })
            }).collect::<Vec<_>>(),
        });
        
        let json = serde_json::to_string_pretty(&export_data)?;
        Ok(json.into_bytes())
    }
    
    async fn export_chapter_anki(&self, book: &Book, chapter: &Chapter) -> Result<Vec<u8>> {
        let mut output = String::new();
        
        output.push_str("#separator:tab\n");
        output.push_str("#html:true\n\n");
        
        let problems = self.db.get_problems_by_chapter(&chapter.id).await?;
        
        for problem in problems {
            if problem.parent_id.is_some() {
                continue;
            }
            
            let front = format!("{} - Задача {}", book.title, problem.number);
            let front_html = format!("<b>{}</b><br><br>{}", 
                front, 
                problem.content.replace("$", "&#36;")
            );
            
            let back_html = if let Some(solution) = self.db.get_solution_for_problem(&problem.id).await? {
                solution.content.replace("$", "&#36;")
            } else {
                "(Решение не добавлено)".to_string()
            };
            
            let tags = format!("{}::chapter_{}", book.id.replace("-", "_"), chapter.number);
            
            output.push_str(&format!("{}\t{}\t{}\n", 
                front_html,
                back_html,
                tags
            ));
        }
        
        Ok(output.into_bytes())
    }
}

/// Export statistics
#[derive(Debug, Clone)]
pub struct ExportStats {
    pub problems_exported: u32,
    pub solutions_exported: u32,
    pub chapters_exported: u32,
    pub formulas_count: u32,
}
