# Project Context: Bookers (`booker-web`)

Last reviewed: 2026-02-08

## TL;DR
This repo is a Rust (Actix-web) web app + CLI to browse PDF/EPUB files, generate page previews, run OCR (math-friendly), parse textbook pages into structured "problems" and "theory blocks", store everything in SQLite, and optionally generate AI solutions (OpenAI/Claude/Mistral). It also supports batch OCR/solve as background jobs with polling + WebSocket progress, and exports (Markdown/LaTeX/JSON/Anki TSV).

## Tech Stack
- Rust: Actix-web server, Tera templates, sqlx (SQLite), tokio.
- Python: `ocr.py` (multi-provider OCR) called from Rust for some endpoints/batch workflows.
- External binaries (system deps): `pdfinfo`, `pdftoppm` (Poppler tools).
- Frontend: server-rendered HTML templates, KaTeX for LaTeX rendering, some vanilla JS; Tailwind used on the index page.

## Key Entry Points
- Server/CLI entry: `src/main.rs`
  - Default: starts web server (`booker serve`).
  - CLI helpers: OCR run / OCR markdown / PDF info.
- Web server bootstrap + routes: `src/server.rs`
- Config/env vars: `src/config/mod.rs`
- Templates: `templates/` (Tera)

## Runtime Storage (Default Paths)
- Input PDFs/EPUBs: `resources/` (configurable via `RESOURCES_DIR`)
- Generated page previews: `resources/.preview/` (configurable via `PREVIEW_DIR`)
- OCR cache (JSON): `resources/.ocr_cache/` (configurable via `OCR_CACHE_DIR`)
- SQLite DB (created on startup): `data/textbooks.db`

## Environment Variables
See `.env.example`. Most used:
- Server: `HOST`, `PORT`, `BASE_URL`
- OCR/AI keys: `MISTRAL_API_KEY`, `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`
- (Python OCR supports more providers: Mathpix/Azure/Google/Kimi, etc.)

## How To Run (Local)
1. Configure env:
   - `cp .env.example .env` and fill keys.
2. Start server:
   - `cargo run` (or `cargo run -- serve`)
3. Open:
   - `http://127.0.0.1:8081/` (default)

Notes:
- Preview generation + PDF metadata require `pdfinfo` and `pdftoppm` to be available in PATH.
- Some OCR flows require the repo-local virtualenv (`.venv`) and Python deps for `ocr.py`.

## Core Flows (High Level)
### 1. File Browser + Preview Generation
- Index page lists `*.pdf` / `*.epub` found under `RESOURCES_DIR` (walkdir).
- `POST /generate_all_previews/{file}`:
  - Uses `pdfinfo` to count pages.
  - Generates page images with `pdftoppm`, stores into `PREVIEW_DIR` as `{file}_<page>.png`.
  - Progress is tracked in-process and polled via `GET /generation_status/{file}`.

### 2. OCR
There are two main OCR implementations:
- Rust -> Mistral OCR API:
  - Handler: `src/handlers/ocr.rs` (`POST /ocr/{file}/{page}`)
  - Provider: `src/services/ocr.rs` (`MistralOcrProvider`)
  - Calls `https://api.mistral.ai/v1/ocr`, stores OCR cache JSON under `OCR_CACHE_DIR`.
  - Saves OCR-returned embedded images into `PREVIEW_DIR` and rewrites markdown image links to `/ocr_image/...`.
- Rust -> Python `ocr.py`:
  - Used by page OCR endpoints + batch processor.
  - Runs `.venv/bin/python ocr.py <image_path> -p <provider>`.
  - `ocr.py` supports multiple providers (Mistral, OpenAI, Claude, Mathpix, Azure, Google, Kimi).

### 3. Parsing OCR Text Into Problems/Theory
Two parsers exist:
- Regex parser: `src/services/parser.rs` (`TextbookParser`)
  - Detects problem starts, theory blocks, sub-problems (a/b/c and Cyrillic variants).
  - Extracts LaTeX formulas for indexing/search.
- Hybrid AI parser: `src/services/ai_parser.rs` (`HybridParser`)
  - Tries AI first (Mistral Large via embedded Python snippet), falls back to regex.
  - Has retry/backoff (`src/services/retry.rs`) and TTL cache keyed by SHA-256 (`src/services/cache.rs`).
  - Adds cross-page flags (`continues_from_prev`, `continues_to_next`).

### 4. Import Into SQLite
- `POST /api/import` (handler: `src/handlers/textbook.rs`):
  - Parses the provided OCR text with `TextbookParser`.
  - Creates/updates `books`, `chapters`, then inserts problems + theory blocks.
- Page-based ingestion also exists via `src/handlers/page_ocr.rs` and stores OCR text into `pages` table.

### 5. Solve Problems With AI
- `POST /api/problems/{problem_id}/solve` (handler: `src/handlers/problems.rs`)
  - Uses `src/services/ai_solver.rs` (`AISolver`) with providers:
    - OpenAI (`/v1/chat/completions`, model `gpt-4o`)
    - Anthropic (`/v1/messages`, model `claude-3-5-sonnet-20241022`)
    - Mistral chat (`mistral-large-latest`)
  - Saves solutions into `solutions` table and updates problem status.

### 6. Batch Processing + Background Jobs
- Batch endpoints: `src/handlers/batch.rs`
  - `POST /api/batch/ocr` (max 100 pages)
  - `POST /api/batch/solve` (max 50 problems)
- In-process job manager: `src/services/background.rs` (`JobManager`)
  - Jobs stored in memory; old completed/failed/cancelled jobs cleaned up periodically.
- WebSocket progress: `GET /ws/jobs` (handler: `src/handlers/websocket.rs`)

### 7. Export
- Handlers: `src/handlers/batch.rs` (`/api/export/book`, `/api/export/chapter/{chapter_id}`)
- Implementation: `src/services/export.rs`
  - Markdown, LaTeX, JSON, and "Anki" (TSV-like export; not a real `.apkg` generator).

## SQLite Schema (What Exists)
Created at startup in `src/services/database.rs`:
- `books`, `chapters`, `pages`
- `problems` (supports sub-problems via `parent_id`, and cross-page columns)
- `theory_blocks`
- `solutions`

## Repo Notes
- There is a legacy/experimental `BookExtractor/` crate (pdfium-based rendering) that appears separate from the current Poppler-based preview pipeline.
- The working tree currently contains many local modifications/untracked files (feature work in progress).

