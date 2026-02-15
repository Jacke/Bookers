use crate::models::problem::{Chapter, Problem, Solution, TheoryBlock, Book};
use anyhow::Result;
use sqlx::{sqlite::SqlitePoolOptions, Pool, Sqlite};

/// Database service for storing and retrieving textbook data
#[derive(Clone)]
pub struct Database {
    pool: Pool<Sqlite>,
}

impl Database {
    /// Create new database connection pool
    pub async fn new(database_url: &str) -> Result<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await?;

        let db = Self { pool };
        db.init().await?;
        
        Ok(db)
    }

    /// Initialize database schema
    async fn init(&self) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS books (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                author TEXT,
                subject TEXT,
                file_path TEXT NOT NULL,
                total_pages INTEGER DEFAULT 0,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE IF NOT EXISTS chapters (
                id TEXT PRIMARY KEY,
                book_id TEXT NOT NULL,
                number INTEGER NOT NULL,
                title TEXT NOT NULL,
                description TEXT,
                problem_count INTEGER DEFAULT 0,
                theory_count INTEGER DEFAULT 0,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE,
                UNIQUE(book_id, number)
            );

            CREATE TABLE IF NOT EXISTS problems (
                id TEXT PRIMARY KEY,
                chapter_id TEXT NOT NULL,
                page_id TEXT, -- References pages(id), NULL if not from OCR
                parent_id TEXT, -- References problems(id) for sub-problems (а, б, в...)
                number TEXT NOT NULL,
                display_name TEXT NOT NULL,
                content TEXT NOT NULL,
                latex_formulas TEXT, -- JSON array
                page_number INTEGER,
                difficulty INTEGER,
                has_solution BOOLEAN DEFAULT FALSE,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                -- Cross-page tracking
                continues_from_page INTEGER, -- Page number if continues from prev page
                continues_to_page INTEGER, -- Page number if continues to next page
                is_cross_page BOOLEAN DEFAULT FALSE, -- True if spans multiple pages
                FOREIGN KEY (chapter_id) REFERENCES chapters(id) ON DELETE CASCADE,
                FOREIGN KEY (page_id) REFERENCES pages(id) ON DELETE SET NULL,
                FOREIGN KEY (parent_id) REFERENCES problems(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_problems_chapter ON problems(chapter_id);
            CREATE INDEX IF NOT EXISTS idx_problems_page ON problems(page_number);
            CREATE INDEX IF NOT EXISTS idx_problems_parent ON problems(parent_id);

            -- Uniqueness rules:
            -- - Main problems: unique per chapter by number
            -- - Sub-problems: unique per parent by letter/number
            CREATE UNIQUE INDEX IF NOT EXISTS uniq_problems_main
              ON problems(chapter_id, number)
              WHERE parent_id IS NULL;
            CREATE UNIQUE INDEX IF NOT EXISTS uniq_problems_sub
              ON problems(parent_id, number)
              WHERE parent_id IS NOT NULL;

            -- Pages table for OCR results
            CREATE TABLE IF NOT EXISTS pages (
                id TEXT PRIMARY KEY,
                book_id TEXT NOT NULL,
                page_number INTEGER NOT NULL,
                ocr_text TEXT,
                has_problems BOOLEAN DEFAULT FALSE,
                problem_count INTEGER DEFAULT 0,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE,
                UNIQUE(book_id, page_number)
            );

            CREATE INDEX IF NOT EXISTS idx_pages_book ON pages(book_id);

            CREATE TABLE IF NOT EXISTS theory_blocks (
                id TEXT PRIMARY KEY,
                chapter_id TEXT NOT NULL,
                block_num INTEGER NOT NULL,
                title TEXT,
                block_type TEXT DEFAULT 'other',
                content TEXT NOT NULL,
                latex_formulas TEXT, -- JSON array
                page_number INTEGER,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (chapter_id) REFERENCES chapters(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_theory_chapter ON theory_blocks(chapter_id);

            CREATE TABLE IF NOT EXISTS solutions (
                id TEXT PRIMARY KEY,
                problem_id TEXT NOT NULL,
                provider TEXT NOT NULL,
                content TEXT NOT NULL,
                latex_formulas TEXT, -- JSON array
                is_verified BOOLEAN DEFAULT FALSE,
                rating INTEGER,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (problem_id) REFERENCES problems(id) ON DELETE CASCADE,
                UNIQUE(problem_id, provider)
            );

            CREATE INDEX IF NOT EXISTS idx_solutions_problem ON solutions(problem_id);

            CREATE TABLE IF NOT EXISTS bookmarks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                problem_id TEXT NOT NULL UNIQUE,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (problem_id) REFERENCES problems(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_bookmarks_problem ON bookmarks(problem_id);
            "#
        )
        .execute(&self.pool)
        .await?;
        
        // Migration: Add cross-page columns if they don't exist
        self.add_cross_page_columns().await?;
        // Migration: legacy schema used a table-level UNIQUE(chapter_id, number) which breaks sub-problems.
        self.migrate_problems_table_uniqueness().await?;
        // Ensure indexes exist after any migration/rebuild.
        self.ensure_problem_indexes().await?;

        Ok(())
    }
    
    /// Migration: Add cross-page columns to existing problems table
    async fn add_cross_page_columns(&self) -> Result<()> {
        // Check if columns exist and add them if not
        let columns = vec![
            ("continues_from_page", "INTEGER"),
            ("continues_to_page", "INTEGER"),
            ("is_cross_page", "BOOLEAN DEFAULT FALSE"),
        ];
        
        for (col, col_type) in columns {
            let exists: bool = sqlx::query_scalar(
                "SELECT COUNT(*) > 0 FROM pragma_table_info('problems') WHERE name = ?1"
            )
            .bind(col)
            .fetch_one(&self.pool)
            .await?;
            
            if !exists {
                sqlx::query(&format!("ALTER TABLE problems ADD COLUMN {} {}", col, col_type))
                    .execute(&self.pool)
                    .await?;
                log::info!("Added column {} to problems table", col);
            }
        }
        
        Ok(())
    }

    /// Ensure indexes/constraints (implemented as indexes) exist on the `problems` table.
    async fn ensure_problem_indexes(&self) -> Result<()> {
        // Split out from the big init SQL so we can re-apply after table rebuilds.
        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_problems_chapter ON problems(chapter_id);
            CREATE INDEX IF NOT EXISTS idx_problems_page ON problems(page_number);
            CREATE INDEX IF NOT EXISTS idx_problems_parent ON problems(parent_id);

            CREATE UNIQUE INDEX IF NOT EXISTS uniq_problems_main
              ON problems(chapter_id, number)
              WHERE parent_id IS NULL;
            CREATE UNIQUE INDEX IF NOT EXISTS uniq_problems_sub
              ON problems(parent_id, number)
              WHERE parent_id IS NOT NULL;
            "#,
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Legacy DBs used `UNIQUE(chapter_id, number)` at the table level which prevents storing multiple
    /// sub-problems like `а)`, `б)` across different parent problems in the same chapter.
    ///
    /// Fix: rebuild the `problems` table without that constraint and use partial unique indexes instead.
    async fn migrate_problems_table_uniqueness(&self) -> Result<()> {
        let table_sql: Option<String> = sqlx::query_scalar(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='problems'",
        )
        .fetch_optional(&self.pool)
        .await?;

        let Some(sql) = table_sql else {
            return Ok(());
        };

        if !sql.to_lowercase().contains("unique(chapter_id, number)") {
            return Ok(());
        }

        log::info!("Migrating legacy problems table uniqueness constraints (enable sub-problems)...");

        let mut tx = self.pool.begin().await?;

        // Rebuild table with foreign keys disabled; we restore them after.
        sqlx::query("PRAGMA foreign_keys = OFF")
            .execute(&mut *tx)
            .await?;

        // Defensive cleanup if a previous migration attempt left artifacts behind.
        sqlx::query("DROP TABLE IF EXISTS problems_new")
            .execute(&mut *tx)
            .await?;

        sqlx::query(
            r#"
            CREATE TABLE problems_new (
                id TEXT PRIMARY KEY,
                chapter_id TEXT NOT NULL,
                page_id TEXT,
                parent_id TEXT,
                number TEXT NOT NULL,
                display_name TEXT NOT NULL,
                content TEXT NOT NULL,
                latex_formulas TEXT,
                page_number INTEGER,
                difficulty INTEGER,
                has_solution BOOLEAN DEFAULT FALSE,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                continues_from_page INTEGER,
                continues_to_page INTEGER,
                is_cross_page BOOLEAN DEFAULT FALSE,
                FOREIGN KEY (chapter_id) REFERENCES chapters(id) ON DELETE CASCADE,
                FOREIGN KEY (page_id) REFERENCES pages(id) ON DELETE SET NULL,
                FOREIGN KEY (parent_id) REFERENCES problems(id) ON DELETE CASCADE
            );
            "#,
        )
        .execute(&mut *tx)
        .await?;

        // Copy data; coalesce nullable columns for newer code expectations.
        sqlx::query(
            r#"
            INSERT INTO problems_new (
                id, chapter_id, page_id, parent_id, number, display_name, content, latex_formulas,
                page_number, difficulty, has_solution, created_at,
                continues_from_page, continues_to_page, is_cross_page
            )
            SELECT
                id, chapter_id, page_id, parent_id, number, display_name, content,
                COALESCE(latex_formulas, '[]'),
                page_number, difficulty, has_solution, created_at,
                continues_from_page, continues_to_page, COALESCE(is_cross_page, 0)
            FROM problems;
            "#,
        )
        .execute(&mut *tx)
        .await?;

        sqlx::query("DROP TABLE problems")
            .execute(&mut *tx)
            .await?;

        sqlx::query("ALTER TABLE problems_new RENAME TO problems")
            .execute(&mut *tx)
            .await?;

        // Recreate indexes on the rebuilt table inside the transaction.
        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_problems_chapter ON problems(chapter_id);
            CREATE INDEX IF NOT EXISTS idx_problems_page ON problems(page_number);
            CREATE INDEX IF NOT EXISTS idx_problems_parent ON problems(parent_id);

            CREATE UNIQUE INDEX IF NOT EXISTS uniq_problems_main
              ON problems(chapter_id, number)
              WHERE parent_id IS NULL;
            CREATE UNIQUE INDEX IF NOT EXISTS uniq_problems_sub
              ON problems(parent_id, number)
              WHERE parent_id IS NOT NULL;
            "#,
        )
        .execute(&mut *tx)
        .await?;

        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;

        Ok(())
    }

    // === Book Operations ===

    pub async fn create_book(&self, book: &Book) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO books (id, title, author, subject, file_path, total_pages)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ON CONFLICT(id) DO UPDATE SET
                title = excluded.title,
                author = excluded.author,
                subject = excluded.subject,
                total_pages = excluded.total_pages
            "#
        )
        .bind(&book.id)
        .bind(&book.title)
        .bind(&book.author)
        .bind(&book.subject)
        .bind(&book.file_path)
        .bind(book.total_pages as i64)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn get_book(&self, id: &str) -> Result<Option<Book>> {
        let row = sqlx::query_as::<_, BookRow>(
            "SELECT * FROM books WHERE id = ?1"
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| r.into()))
    }

    pub async fn list_books(&self) -> Result<Vec<Book>> {
        let rows = sqlx::query_as::<_, BookRow>(
            "SELECT * FROM books ORDER BY created_at DESC"
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    // === Chapter Operations ===

    pub async fn create_chapter(&self, chapter: &Chapter) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO chapters (id, book_id, number, title, description, problem_count, theory_count)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ON CONFLICT(id) DO UPDATE SET
                title = excluded.title,
                description = excluded.description
            "#
        )
        .bind(&chapter.id)
        .bind(&chapter.book_id)
        .bind(chapter.number as i64)
        .bind(&chapter.title)
        .bind(&chapter.description)
        .bind(chapter.problem_count as i64)
        .bind(chapter.theory_count as i64)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn get_chapter(&self, id: &str) -> Result<Option<Chapter>> {
        let row = sqlx::query_as::<_, ChapterRow>(
            "SELECT * FROM chapters WHERE id = ?1"
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| r.into()))
    }

    pub async fn get_chapters_by_book(&self, book_id: &str) -> Result<Vec<Chapter>> {
        let rows = sqlx::query_as::<_, ChapterRow>(
            "SELECT * FROM chapters WHERE book_id = ?1 ORDER BY number"
        )
        .bind(book_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    // === Problem Operations ===

    pub async fn create_problem(&self, problem: &Problem) -> Result<()> {
        let formulas_json = serde_json::to_string(&problem.latex_formulas)?;
        
        // Determine if cross-page
        let is_cross_page = problem.continues_from_page.is_some() || problem.continues_to_page.is_some();
        
        // Upsert by primary key to avoid DELETE+INSERT semantics (which would cascade-delete solutions).
        // Uniqueness for main problems and sub-problems is enforced via partial unique indexes.
        sqlx::query(
            r#"
            INSERT INTO problems 
            (id, chapter_id, page_id, parent_id, number, display_name, content, latex_formulas, 
             page_number, difficulty, has_solution, continues_from_page, continues_to_page, is_cross_page)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
            ON CONFLICT(id) DO UPDATE SET
                chapter_id = excluded.chapter_id,
                page_id = excluded.page_id,
                parent_id = excluded.parent_id,
                number = excluded.number,
                display_name = excluded.display_name,
                content = excluded.content,
                latex_formulas = excluded.latex_formulas,
                page_number = excluded.page_number,
                difficulty = excluded.difficulty,
                -- Keep has_solution as-is (don't wipe user-generated data)
                continues_from_page = excluded.continues_from_page,
                continues_to_page = excluded.continues_to_page,
                is_cross_page = excluded.is_cross_page
            "#
        )
        .bind(&problem.id)
        .bind(&problem.chapter_id)
        .bind(&problem.page_id)
        .bind(&problem.parent_id)
        .bind(&problem.number)
        .bind(&problem.display_name)
        .bind(&problem.content)
        .bind(formulas_json)
        .bind(problem.page_number.map(|p| p as i64))
        .bind(problem.difficulty.map(|d| d as i64))
        .bind(problem.has_solution)
        .bind(problem.continues_from_page.map(|p| p as i64))
        .bind(problem.continues_to_page.map(|p| p as i64))
        .bind(is_cross_page)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn get_problem(&self, id: &str) -> Result<Option<Problem>> {
        let row = sqlx::query_as::<_, ProblemRow>(
            "SELECT * FROM problems WHERE id = ?1"
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| r.into()))
    }

    pub async fn get_problems_by_chapter(&self, chapter_id: &str) -> Result<Vec<Problem>> {
        let rows = sqlx::query_as::<_, ProblemRow>(
            "SELECT * FROM problems WHERE chapter_id = ?1 AND parent_id IS NULL ORDER BY number"
        )
        .bind(chapter_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    /// Delete all problems (and sub-problems) for a page
    pub async fn delete_problems_by_page(&self, page_id: &str) -> Result<usize> {
        // First delete sub-problems (they reference parent problems)
        let sub_count = sqlx::query(
            "DELETE FROM problems WHERE parent_id IN (SELECT id FROM problems WHERE page_id = ?1)"
        )
        .bind(page_id)
        .execute(&self.pool)
        .await?
        .rows_affected();
        
        // Then delete parent problems
        let parent_count = sqlx::query(
            "DELETE FROM problems WHERE page_id = ?1"
        )
        .bind(page_id)
        .execute(&self.pool)
        .await?
        .rows_affected();
        
        Ok((sub_count + parent_count) as usize)
    }

    /// Create or update multiple problems at once
    pub async fn create_or_update_problems(&self, problems: &[Problem]) -> Result<usize> {
        let mut count = 0;
        for problem in problems {
            self.create_problem(problem).await?;
            count += 1;
        }
        Ok(count)
    }

    pub async fn update_problem_solution_status(&self, problem_id: &str, has_solution: bool) -> Result<()> {
        sqlx::query(
            "UPDATE problems SET has_solution = ?1 WHERE id = ?2"
        )
        .bind(has_solution)
        .bind(problem_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Update problem content and latex formulas (e.g., after OCR import)
    pub async fn update_problem_content(&self, problem_id: &str, content: &str, latex_formulas: Vec<String>) -> Result<()> {
        let formulas_json = serde_json::to_string(&latex_formulas)?;
        
        sqlx::query(
            "UPDATE problems SET content = ?1, latex_formulas = ?2 WHERE id = ?3"
        )
        .bind(content)
        .bind(formulas_json)
        .bind(problem_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    // === Page Operations ===

    pub async fn get_or_create_page(&self, book_id: &str, page_number: u32) -> Result<crate::models::Page> {
        let page_id = format!("{}:page:{}", book_id, page_number);
        
        // Try to get existing page
        let existing = sqlx::query_as::<_, PageRow>(
            "SELECT * FROM pages WHERE id = ?1"
        )
        .bind(&page_id)
        .fetch_optional(&self.pool)
        .await?;
        
        if let Some(row) = existing {
            return Ok(row.into());
        }
        
        // Ensure book exists first
        let book = crate::models::Book {
            id: book_id.to_string(),
            title: book_id.to_string(),
            author: None,
            subject: None,
            file_path: format!("resources/{}.pdf", book_id),
            total_pages: 0,
            created_at: chrono::Utc::now(),
        };
        
        // Try to create book (ignore if exists)
        if let Err(e) = self.create_book(&book).await {
            log::debug!("Book may already exist: {}", e);
        }
        
        // Create new page
        let now = chrono::Utc::now();
        let page = crate::models::Page {
            id: page_id.clone(),
            book_id: book_id.to_string(),
            page_number,
            ocr_text: None,
            has_problems: false,
            problem_count: 0,
            created_at: now,
            updated_at: now,
        };
        
        sqlx::query(
            "INSERT INTO pages (id, book_id, page_number, ocr_text, has_problems, problem_count) VALUES (?1, ?2, ?3, ?4, ?5, ?6)"
        )
        .bind(&page.id)
        .bind(&page.book_id)
        .bind(page_number as i64)
        .bind(&page.ocr_text)
        .bind(page.has_problems)
        .bind(page.problem_count as i64)
        .execute(&self.pool)
        .await?;
        
        Ok(page)
    }

    pub async fn update_page_ocr(&self, page_id: &str, ocr_text: &str, problem_count: u32) -> Result<()> {
        sqlx::query(
            "UPDATE pages SET ocr_text = ?1, has_problems = ?2, problem_count = ?3, updated_at = CURRENT_TIMESTAMP WHERE id = ?4"
        )
        .bind(ocr_text)
        .bind(problem_count > 0)
        .bind(problem_count as i64)
        .bind(page_id)
        .execute(&self.pool)
        .await?;
        
        Ok(())
    }

    pub async fn get_problems_by_page(&self, page_id: &str) -> Result<Vec<Problem>> {
        // Only get parent problems (not sub-problems)
        let rows = sqlx::query_as::<_, ProblemRow>(
            "SELECT * FROM problems WHERE page_id = ?1 AND parent_id IS NULL ORDER BY number"
        )
        .bind(page_id)
        .fetch_all(&self.pool)
        .await?;
        
        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    /// Get sub-problems for a parent problem
    pub async fn get_sub_problems(&self, parent_id: &str) -> Result<Vec<Problem>> {
        let rows = sqlx::query_as::<_, ProblemRow>(
            "SELECT * FROM problems WHERE parent_id = ?1 ORDER BY number"
        )
        .bind(parent_id)
        .fetch_all(&self.pool)
        .await?;
        
        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    /// Get problem with sub-problems loaded
    pub async fn get_problem_with_subs(&self, id: &str) -> Result<Option<Problem>> {
        let mut problem = match self.get_problem(id).await? {
            Some(p) => p,
            None => return Ok(None),
        };
        
        let subs = self.get_sub_problems(id).await?;
        if !subs.is_empty() {
            problem.sub_problems = Some(subs);
        }
        
        Ok(Some(problem))
    }

    pub async fn get_page(&self, book_id: &str, page_number: u32) -> Result<Option<crate::models::Page>> {
        let page_id = format!("{}:page:{}", book_id, page_number);
        let row = sqlx::query_as::<_, PageRow>(
            "SELECT * FROM pages WHERE id = ?1"
        )
        .bind(&page_id)
        .fetch_optional(&self.pool)
        .await?;
        
        Ok(row.map(|r| r.into()))
    }

    /// Get all pages for a book
    pub async fn get_pages_by_book(&self, book_id: &str) -> Result<Vec<crate::models::Page>> {
        let rows = sqlx::query_as::<_, PageRow>(
            "SELECT * FROM pages WHERE book_id = ?1 ORDER BY page_number"
        )
        .bind(book_id)
        .fetch_all(&self.pool)
        .await?;
        
        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    // === Theory Operations ===

    pub async fn create_theory_block(&self, theory: &TheoryBlock) -> Result<()> {
        let formulas_json = serde_json::to_string(&theory.latex_formulas)?;
        let block_type = format!("{:?}", theory.block_type).to_lowercase();
        
        sqlx::query(
            r#"
            INSERT INTO theory_blocks (id, chapter_id, block_num, title, block_type, content, latex_formulas, page_number)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            ON CONFLICT(id) DO UPDATE SET
                content = excluded.content,
                latex_formulas = excluded.latex_formulas
            "#
        )
        .bind(&theory.id)
        .bind(&theory.chapter_id)
        .bind(theory.block_num as i64)
        .bind(&theory.title)
        .bind(block_type)
        .bind(&theory.content)
        .bind(formulas_json)
        .bind(theory.page_number.map(|p| p as i64))
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn get_theory_blocks_by_chapter(&self, chapter_id: &str) -> Result<Vec<TheoryBlock>> {
        let rows = sqlx::query_as::<_, TheoryRow>(
            "SELECT * FROM theory_blocks WHERE chapter_id = ?1 ORDER BY block_num"
        )
        .bind(chapter_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    // === Solution Operations ===

    pub async fn create_or_update_solution(&self, solution: &Solution) -> Result<()> {
        let formulas_json = serde_json::to_string(&solution.latex_formulas)?;
        
        sqlx::query(
            r#"
            INSERT INTO solutions (id, problem_id, provider, content, latex_formulas, is_verified, rating, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, CURRENT_TIMESTAMP)
            ON CONFLICT(problem_id, provider) DO UPDATE SET
                content = excluded.content,
                latex_formulas = excluded.latex_formulas,
                updated_at = CURRENT_TIMESTAMP
            "#
        )
        .bind(&solution.id)
        .bind(&solution.problem_id)
        .bind(&solution.provider)
        .bind(&solution.content)
        .bind(formulas_json)
        .bind(solution.is_verified)
        .bind(solution.rating.map(|r| r as i64))
        .execute(&self.pool)
        .await?;

        // Update problem's has_solution flag
        sqlx::query(
            "UPDATE problems SET has_solution = TRUE WHERE id = ?1"
        )
        .bind(&solution.problem_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn get_solution(&self, problem_id: &str, provider: &str) -> Result<Option<Solution>> {
        let row = sqlx::query_as::<_, SolutionRow>(
            "SELECT * FROM solutions WHERE problem_id = ?1 AND provider = ?2"
        )
        .bind(problem_id)
        .bind(provider)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| r.into()))
    }

    pub async fn get_solutions_by_problem(&self, problem_id: &str) -> Result<Vec<Solution>> {
        let rows = sqlx::query_as::<_, SolutionRow>(
            "SELECT * FROM solutions WHERE problem_id = ?1 ORDER BY created_at DESC"
        )
        .bind(problem_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    pub async fn rate_solution(&self, solution_id: &str, rating: u8) -> Result<()> {
        sqlx::query(
            "UPDATE solutions SET rating = ?1 WHERE id = ?2"
        )
        .bind(rating as i64)
        .bind(solution_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn verify_solution(&self, solution_id: &str, verified: bool) -> Result<()> {
        sqlx::query(
            "UPDATE solutions SET is_verified = ?1 WHERE id = ?2"
        )
        .bind(verified)
        .bind(solution_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }
    
    /// Get any solution for a problem (prefer verified, then highest rated)
    pub async fn get_solution_for_problem(&self, problem_id: &str) -> Result<Option<Solution>> {
        let row = sqlx::query_as::<_, SolutionRow>(
            r#"SELECT * FROM solutions 
               WHERE problem_id = ?1 
               ORDER BY is_verified DESC, rating DESC NULLS LAST, created_at DESC 
               LIMIT 1"#
        )
        .bind(problem_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| r.into()))
    }
    
    /// Save or update solution
    pub async fn save_solution(&self, solution: &Solution) -> Result<()> {
        let formulas_json = serde_json::to_string(&solution.latex_formulas)?;
        
        sqlx::query(
            r#"INSERT INTO solutions 
               (id, problem_id, provider, content, latex_formulas, is_verified, rating, created_at, updated_at)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
               ON CONFLICT(problem_id, provider) DO UPDATE SET
                   content = excluded.content,
                   latex_formulas = excluded.latex_formulas,
                   updated_at = excluded.updated_at"#
        )
        .bind(&solution.id)
        .bind(&solution.problem_id)
        .bind(&solution.provider)
        .bind(&solution.content)
        .bind(formulas_json)
        .bind(solution.is_verified)
        .bind(solution.rating.map(|r| r as i64))
        .bind(solution.created_at)
        .bind(solution.updated_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Add a problem to bookmarks
    pub async fn add_bookmark(&self, problem_id: &str) -> Result<()> {
        sqlx::query(
            "INSERT OR IGNORE INTO bookmarks (problem_id) VALUES (?1)"
        )
        .bind(problem_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Remove a problem from bookmarks
    pub async fn remove_bookmark(&self, problem_id: &str) -> Result<()> {
        sqlx::query(
            "DELETE FROM bookmarks WHERE problem_id = ?1"
        )
        .bind(problem_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get all bookmarked problems
    pub async fn get_bookmarked_problems(&self) -> Result<Vec<Problem>> {
        let rows = sqlx::query_as::<_, ProblemRow>(
            r#"SELECT p.* FROM problems p
               INNER JOIN bookmarks b ON p.id = b.problem_id
               ORDER BY b.created_at DESC"#
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    /// Check if a problem is bookmarked
    pub async fn is_bookmarked(&self, problem_id: &str) -> Result<bool> {
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT 1 FROM bookmarks WHERE problem_id = ?1"
        )
        .bind(problem_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.is_some())
    }

    // === Search Operations ===

    pub async fn search_by_formula(&self, formula: &str, limit: usize) -> Result<Vec<Problem>> {
        let pattern = format!("%{}%", formula);
        let rows = sqlx::query_as::<_, ProblemRow>(
            "SELECT * FROM problems WHERE latex_formulas LIKE ?1 LIMIT ?2"
        )
        .bind(pattern)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }
}

// === Database Row Types ===

#[derive(sqlx::FromRow)]
struct BookRow {
    id: String,
    title: String,
    author: Option<String>,
    subject: Option<String>,
    file_path: String,
    total_pages: i64,
    created_at: chrono::NaiveDateTime,
}

impl From<BookRow> for Book {
    fn from(row: BookRow) -> Self {
        Self {
            id: row.id,
            title: row.title,
            author: row.author,
            subject: row.subject,
            file_path: row.file_path,
            total_pages: row.total_pages as u32,
            created_at: chrono::DateTime::from_naive_utc_and_offset(row.created_at, chrono::Utc),
        }
    }
}

#[derive(sqlx::FromRow)]
struct ChapterRow {
    id: String,
    book_id: String,
    number: i64,
    title: String,
    description: Option<String>,
    problem_count: i64,
    theory_count: i64,
    created_at: chrono::NaiveDateTime,
}

impl From<ChapterRow> for Chapter {
    fn from(row: ChapterRow) -> Self {
        Self {
            id: row.id,
            book_id: row.book_id,
            number: row.number as u32,
            title: row.title,
            description: row.description,
            problem_count: row.problem_count as u32,
            theory_count: row.theory_count as u32,
            created_at: chrono::DateTime::from_naive_utc_and_offset(row.created_at, chrono::Utc),
        }
    }
}

#[derive(sqlx::FromRow)]
struct ProblemRow {
    id: String,
    chapter_id: String,
    page_id: Option<String>,
    parent_id: Option<String>,
    number: String,
    display_name: String,
    content: String,
    latex_formulas: String,
    page_number: Option<i64>,
    difficulty: Option<i64>,
    has_solution: bool,
    created_at: chrono::NaiveDateTime,
    continues_from_page: Option<i64>,
    continues_to_page: Option<i64>,
    is_cross_page: Option<bool>,
}

impl From<ProblemRow> for Problem {
    fn from(row: ProblemRow) -> Self {
        let formulas: Vec<String> = serde_json::from_str(&row.latex_formulas).unwrap_or_default();
        
        Self {
            id: row.id,
            chapter_id: row.chapter_id,
            page_id: row.page_id,
            parent_id: row.parent_id,
            number: row.number,
            display_name: row.display_name,
            content: row.content,
            latex_formulas: formulas,
            page_number: row.page_number.map(|p| p as u32),
            difficulty: row.difficulty.map(|d| d as u8),
            has_solution: row.has_solution,
            created_at: chrono::DateTime::from_naive_utc_and_offset(row.created_at, chrono::Utc),
            solution: None,
            sub_problems: None,
            continues_from_page: row.continues_from_page.map(|p| p as u32),
            continues_to_page: row.continues_to_page.map(|p| p as u32),
            is_cross_page: row.is_cross_page.unwrap_or(false),
        }
    }
}

#[derive(sqlx::FromRow)]
struct PageRow {
    id: String,
    book_id: String,
    page_number: i64,
    ocr_text: Option<String>,
    has_problems: bool,
    problem_count: i64,
    created_at: chrono::NaiveDateTime,
    updated_at: chrono::NaiveDateTime,
}

impl From<PageRow> for crate::models::Page {
    fn from(row: PageRow) -> Self {
        Self {
            id: row.id,
            book_id: row.book_id,
            page_number: row.page_number as u32,
            ocr_text: row.ocr_text,
            has_problems: row.has_problems,
            problem_count: row.problem_count as u32,
            created_at: chrono::DateTime::from_naive_utc_and_offset(row.created_at, chrono::Utc),
            updated_at: chrono::DateTime::from_naive_utc_and_offset(row.updated_at, chrono::Utc),
        }
    }
}

#[derive(sqlx::FromRow)]
struct TheoryRow {
    id: String,
    chapter_id: String,
    block_num: i64,
    title: Option<String>,
    block_type: String,
    content: String,
    latex_formulas: String,
    page_number: Option<i64>,
    created_at: chrono::NaiveDateTime,
}

impl From<TheoryRow> for TheoryBlock {
    fn from(row: TheoryRow) -> Self {
        let formulas: Vec<String> = serde_json::from_str(&row.latex_formulas).unwrap_or_default();
        let block_type = match row.block_type.as_str() {
            "definition" => crate::models::problem::TheoryType::Definition,
            "theorem" => crate::models::problem::TheoryType::Theorem,
            "proof" => crate::models::problem::TheoryType::Proof,
            "property" => crate::models::problem::TheoryType::Property,
            "formula" => crate::models::problem::TheoryType::Formula,
            "example" => crate::models::problem::TheoryType::Example,
            _ => crate::models::problem::TheoryType::Other,
        };

        Self {
            id: row.id,
            chapter_id: row.chapter_id,
            block_num: row.block_num as u32,
            title: row.title,
            block_type,
            content: row.content,
            latex_formulas: formulas,
            page_number: row.page_number.map(|p| p as u32),
            created_at: chrono::DateTime::from_naive_utc_and_offset(row.created_at, chrono::Utc),
        }
    }
}

#[derive(sqlx::FromRow)]
struct SolutionRow {
    id: String,
    problem_id: String,
    provider: String,
    content: String,
    latex_formulas: String,
    is_verified: bool,
    rating: Option<i64>,
    created_at: chrono::NaiveDateTime,
    updated_at: chrono::NaiveDateTime,
}

impl From<SolutionRow> for Solution {
    fn from(row: SolutionRow) -> Self {
        let formulas: Vec<String> = serde_json::from_str(&row.latex_formulas).unwrap_or_default();
        
        Self {
            id: row.id,
            problem_id: row.problem_id,
            provider: row.provider,
            content: row.content,
            latex_formulas: formulas,
            is_verified: row.is_verified,
            rating: row.rating.map(|r| r as u8),
            created_at: chrono::DateTime::from_naive_utc_and_offset(row.created_at, chrono::Utc),
            updated_at: chrono::DateTime::from_naive_utc_and_offset(row.updated_at, chrono::Utc),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::Connection;

    async fn new_temp_db() -> (Database, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!("bookers_test_{}.db", uuid::Uuid::new_v4()));
        // Ensure the file exists so the URL is always valid.
        let _ = std::fs::File::create(&path);
        let url = format!("sqlite:{}", path.to_str().unwrap());
        let db = Database::new(&url).await.expect("db init");
        (db, path)
    }

    async fn seed_book_and_chapter(db: &Database, book_id: &str, chapter_num: u32) -> String {
        let book = Book {
            id: book_id.to_string(),
            title: book_id.to_string(),
            author: None,
            subject: None,
            file_path: format!("resources/{}.pdf", book_id),
            total_pages: 0,
            created_at: chrono::Utc::now(),
        };
        db.create_book(&book).await.expect("create book");

        let chapter_id = format!("{}:{}", book_id, chapter_num);
        let chapter = Chapter {
            id: chapter_id.clone(),
            book_id: book_id.to_string(),
            number: chapter_num,
            title: format!("Глава {}", chapter_num),
            description: None,
            problem_count: 0,
            theory_count: 0,
            created_at: chrono::Utc::now(),
        };
        db.create_chapter(&chapter).await.expect("create chapter");
        chapter_id
    }

    #[tokio::test]
    async fn sub_problems_can_repeat_letters_across_different_parents() {
        let (db, path) = new_temp_db().await;
        let chapter_id = seed_book_and_chapter(&db, "algebra-7", 1).await;

        let p1_id = Problem::generate_id("algebra-7", 1, "71");
        let p2_id = Problem::generate_id("algebra-7", 1, "72");

        let now = chrono::Utc::now();
        let problems = vec![
            Problem {
                id: p1_id.clone(),
                chapter_id: chapter_id.clone(),
                page_id: None,
                parent_id: None,
                number: "71".to_string(),
                display_name: "Задача 71".to_string(),
                content: "71. Foo".to_string(),
                latex_formulas: vec![],
                page_number: Some(1),
                difficulty: None,
                has_solution: false,
                created_at: now,
                solution: None,
                sub_problems: None,
                continues_from_page: None,
                continues_to_page: None,
                is_cross_page: false,
            },
            Problem {
                id: p2_id.clone(),
                chapter_id: chapter_id.clone(),
                page_id: None,
                parent_id: None,
                number: "72".to_string(),
                display_name: "Задача 72".to_string(),
                content: "72. Bar".to_string(),
                latex_formulas: vec![],
                page_number: Some(1),
                difficulty: None,
                has_solution: false,
                created_at: now,
                solution: None,
                sub_problems: None,
                continues_from_page: None,
                continues_to_page: None,
                is_cross_page: false,
            },
            Problem {
                id: format!("{}:a", p1_id),
                chapter_id: chapter_id.clone(),
                page_id: None,
                parent_id: Some(p1_id.clone()),
                number: "a".to_string(),
                display_name: "a)".to_string(),
                content: "a) sub 1".to_string(),
                latex_formulas: vec![],
                page_number: Some(1),
                difficulty: None,
                has_solution: false,
                created_at: now,
                solution: None,
                sub_problems: None,
                continues_from_page: None,
                continues_to_page: None,
                is_cross_page: false,
            },
            Problem {
                id: format!("{}:a", p2_id),
                chapter_id: chapter_id.clone(),
                page_id: None,
                parent_id: Some(p2_id.clone()),
                number: "a".to_string(),
                display_name: "a)".to_string(),
                content: "a) sub 2".to_string(),
                latex_formulas: vec![],
                page_number: Some(1),
                difficulty: None,
                has_solution: false,
                created_at: now,
                solution: None,
                sub_problems: None,
                continues_from_page: None,
                continues_to_page: None,
                is_cross_page: false,
            },
        ];

        db.create_or_update_problems(&problems)
            .await
            .expect("insert problems");

        // Chapter listing should return only parent problems.
        let parents = db.get_problems_by_chapter(&chapter_id).await.expect("list");
        assert_eq!(parents.len(), 2);

        let subs1 = db.get_sub_problems(&p1_id).await.expect("subs1");
        let subs2 = db.get_sub_problems(&p2_id).await.expect("subs2");
        assert_eq!(subs1.len(), 1);
        assert_eq!(subs2.len(), 1);
        assert_eq!(subs1[0].number, "a");
        assert_eq!(subs2[0].number, "a");

        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn migrates_legacy_unique_constraint_and_allows_sub_problems() {
        let path = std::env::temp_dir().join(format!("bookers_test_legacy_{}.db", uuid::Uuid::new_v4()));
        let _ = std::fs::File::create(&path);
        let url = format!("sqlite:{}", path.to_str().unwrap());

        // Create a legacy `problems` table with table-level UNIQUE(chapter_id, number).
        let mut conn = sqlx::SqliteConnection::connect(&url).await.expect("connect");
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS problems (
                id TEXT PRIMARY KEY,
                chapter_id TEXT NOT NULL,
                page_id TEXT,
                parent_id TEXT,
                number TEXT NOT NULL,
                display_name TEXT NOT NULL,
                content TEXT NOT NULL,
                latex_formulas TEXT,
                page_number INTEGER,
                difficulty INTEGER,
                has_solution BOOLEAN DEFAULT FALSE,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (chapter_id) REFERENCES chapters(id) ON DELETE CASCADE,
                FOREIGN KEY (page_id) REFERENCES pages(id) ON DELETE SET NULL,
                FOREIGN KEY (parent_id) REFERENCES problems(id) ON DELETE CASCADE,
                UNIQUE(chapter_id, number)
            );
            "#,
        )
        .execute(&mut conn)
        .await
        .expect("create legacy problems");
        drop(conn);

        // Init with current code: should rebuild the table and remove the legacy UNIQUE constraint.
        let db = Database::new(&url).await.expect("init db");

        let sql: String = sqlx::query_scalar(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='problems'",
        )
        .fetch_one(&db.pool)
        .await
        .expect("read schema");
        assert!(
            !sql.to_lowercase().contains("unique(chapter_id, number)"),
            "legacy UNIQUE constraint should be removed"
        );

        let chapter_id = seed_book_and_chapter(&db, "algebra-7", 1).await;

        // Now ensure repeated letters across parents are accepted.
        let p1_id = Problem::generate_id("algebra-7", 1, "71");
        let p2_id = Problem::generate_id("algebra-7", 1, "72");

        let now = chrono::Utc::now();
        let problems = vec![
            Problem {
                id: p1_id.clone(),
                chapter_id: chapter_id.clone(),
                page_id: None,
                parent_id: None,
                number: "71".to_string(),
                display_name: "Задача 71".to_string(),
                content: "71. Foo".to_string(),
                latex_formulas: vec![],
                page_number: Some(1),
                difficulty: None,
                has_solution: false,
                created_at: now,
                solution: None,
                sub_problems: None,
                continues_from_page: None,
                continues_to_page: None,
                is_cross_page: false,
            },
            Problem {
                id: p2_id.clone(),
                chapter_id: chapter_id.clone(),
                page_id: None,
                parent_id: None,
                number: "72".to_string(),
                display_name: "Задача 72".to_string(),
                content: "72. Bar".to_string(),
                latex_formulas: vec![],
                page_number: Some(1),
                difficulty: None,
                has_solution: false,
                created_at: now,
                solution: None,
                sub_problems: None,
                continues_from_page: None,
                continues_to_page: None,
                is_cross_page: false,
            },
            Problem {
                id: format!("{}:a", p1_id),
                chapter_id: chapter_id.clone(),
                page_id: None,
                parent_id: Some(p1_id.clone()),
                number: "a".to_string(),
                display_name: "a)".to_string(),
                content: "a) sub 1".to_string(),
                latex_formulas: vec![],
                page_number: Some(1),
                difficulty: None,
                has_solution: false,
                created_at: now,
                solution: None,
                sub_problems: None,
                continues_from_page: None,
                continues_to_page: None,
                is_cross_page: false,
            },
            Problem {
                id: format!("{}:a", p2_id),
                chapter_id: chapter_id.clone(),
                page_id: None,
                parent_id: Some(p2_id.clone()),
                number: "a".to_string(),
                display_name: "a)".to_string(),
                content: "a) sub 2".to_string(),
                latex_formulas: vec![],
                page_number: Some(1),
                difficulty: None,
                has_solution: false,
                created_at: now,
                solution: None,
                sub_problems: None,
                continues_from_page: None,
                continues_to_page: None,
                is_cross_page: false,
            },
        ];

        db.create_or_update_problems(&problems)
            .await
            .expect("insert problems after migration");

        let subs1 = db.get_sub_problems(&p1_id).await.expect("subs1");
        let subs2 = db.get_sub_problems(&p2_id).await.expect("subs2");
        assert_eq!(subs1.len(), 1);
        assert_eq!(subs2.len(), 1);

        let _ = std::fs::remove_file(path);
    }
}
