//! PDF parse gate: run pure-Rust `pdf-extract` over every downloaded PDF, write
//! the text to data/txt_rust/, and print per-doc stats so we can judge quality
//! (char count, and whether numbered legislative structure survived extraction).
use rag_parse::extract_text;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

fn find_pdfs(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(committees) = fs::read_dir(root) {
        for c in committees.flatten() {
            if let Ok(files) = fs::read_dir(c.path()) {
                for f in files.flatten() {
                    let p = f.path();
                    if p.extension().map(|e| e == "pdf").unwrap_or(false) {
                        out.push(p);
                    }
                }
            }
        }
    }
    out.sort();
    out
}

/// Cheap structure probe: how many "Article N" / numbered-paragraph markers survive.
fn structure_markers(text: &str) -> usize {
    text.match_indices("Article ").count()
        + text.match_indices("Amendment ").count()
        + text.match_indices("Recital").count()
}

fn main() -> anyhow::Result<()> {
    let pdfs = find_pdfs(Path::new("data/pdfs"));
    let out_dir = Path::new("data/txt_rust");
    fs::create_dir_all(out_dir)?;
    println!("pdf-extract over {} PDFs\n", pdfs.len());
    println!("{:<20} {:>8} {:>8} {:>7} {:>8}", "doc", "chars", "lines", "struct", "ms");

    for pdf in &pdfs {
        let stem = pdf.file_stem().unwrap().to_string_lossy().to_string();
        let t = Instant::now();
        match extract_text(pdf) {
            Ok(text) => {
                let ms = t.elapsed().as_millis();
                fs::write(out_dir.join(format!("{stem}.txt")), &text)?;
                println!(
                    "{:<20} {:>8} {:>8} {:>7} {:>8}",
                    stem,
                    text.len(),
                    text.lines().count(),
                    structure_markers(&text),
                    ms
                );
            }
            Err(e) => println!("{stem:<20} FAILED: {e}"),
        }
    }
    println!("\ntext written to {}/", out_dir.display());
    Ok(())
}
