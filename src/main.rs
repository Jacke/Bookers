mod cli;
mod config;
mod error;
mod handlers;
mod models;
mod server;
mod services;
mod utils;

use clap::Parser;
use cli::{Cli, Commands};

fn main() {
    dotenvy::dotenv().ok();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let cli = Cli::parse();

    match &cli.command {
        Some(Commands::Serve) | None => {
            actix_web::rt::System::new()
                .block_on(server::run())
                .expect("Server failed to start");
        }
        Some(Commands::OcrMarkdown { file, page }) => {
            cli::handle_ocr_markdown(file, page);
        }
        Some(Commands::OcrRun { file, page }) => {
            cli::handle_ocr_run(file, page);
        }
        Some(Commands::PdfInfo { file }) => {
            cli::handle_pdf_info(file);
        }
    }
}
