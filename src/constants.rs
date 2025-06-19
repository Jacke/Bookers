// src/constants.rs

pub const RESOURCES_DIR: &str = "./resources";
pub const PREVIEW_DIR: &str = "./resources/.preview";
pub const OCR_CACHE_DIR: &str = "./resources/.ocr_cache";

// Шаблоны имён файлов
pub const PREVIEW_IMAGE_PATTERN: &str = "{file_stem}_{page}.png";
pub const OCR_IMAGE_PATTERN: &str = "ocr_image-{provider_id}-{file_stem}-{page}-img-{img_index}.{ext}";
pub const OCR_CACHE_PATTERN: &str = "{file_stem}_{page}.ocr_cache";
pub const OCR_RAW_JSON_PATTERN: &str = "{file_stem}-{page}.json";

// Поддерживаемые расширения
pub const SUPPORTED_EXTENSIONS: &[&str] = &["pdf", "epub"];
pub const PREVIEW_IMAGE_EXT: &str = "png";
pub const OCR_IMAGE_EXTS: &[&str] = &["jpeg", "jpg", "png"];
pub const OCR_CACHE_EXT: &str = "ocr_cache";
pub const OCR_RAW_JSON_EXT: &str = "json"; 