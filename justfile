set shell := ["bash", "-eu", "-o", "pipefail", "-c"]
set dotenv-load := true

# Show available commands and their descriptions.
default:
  @just --list

# Start the web server (reads HOST/PORT from .env if set).
serve:
  cargo run -- serve

# Start the app using the default CLI path (same as `serve`).
run:
  cargo run

# Build the project binaries.
build:
  cargo build

# Check compilation without building binaries.
check:
  cargo check

# Run all test suites.
test:
  cargo test

# Run all tests with quieter output.
test-q:
  cargo test -q

# Format all Rust code.
fmt:
  cargo fmt --all

# Run clippy lints for all targets.
clippy:
  cargo clippy --all-targets

# Print CLI help with subcommands and options.
help:
  cargo run -- --help

# Show PDF metadata (pages, dimensions, author, etc.).
pdf-info file:
  cargo run -- pdf-info "{{file}}"

# Run OCR for file and page/range and print raw output.
ocr-run file page:
  cargo run -- ocr-run "{{file}}" "{{page}}"

# Print OCR markdown from cache (or run OCR if cache is missing).
ocr-markdown file page:
  cargo run -- ocr-markdown "{{file}}" "{{page}}"

# Run legacy import smoke test script.
test-import:
  bash ./test_import.sh

# Run page-aware import smoke test script.
test-pages:
  bash ./test_with_pages.sh

# Verify local server health endpoint.
health:
  curl -fsS http://127.0.0.1:8081/healthz

# Scan repository files for secrets with gitleaks.
secrets-gitleaks:
  gitleaks detect --source . --no-git --redact

# Scan repository files for verified secrets with trufflehog.
secrets-trufflehog:
  trufflehog filesystem . --only-verified

# Print local SQLite schema for the `problems` table.
db-schema:
  sqlite3 data/textbooks.db "SELECT sql FROM sqlite_master WHERE type='table' AND name='problems';"

# Print local SQLite indexes for the `problems` table.
db-indexes:
  sqlite3 data/textbooks.db "SELECT name, sql FROM sqlite_master WHERE type='index' AND tbl_name='problems' ORDER BY name;"
