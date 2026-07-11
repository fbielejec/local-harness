//! PDF text extraction (pure Rust). One function; the gate bin measures quality.
use anyhow::Result;
use std::path::Path;

/// Extract the full text of a PDF with pure-Rust `pdf-extract`.
pub fn extract_text(path: &Path) -> Result<String> {
    Ok(pdf_extract::extract_text(path)?)
}
