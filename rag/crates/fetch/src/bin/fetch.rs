//! `cargo run -p ep-rag-fetch --bin fetch -- [per_type_limit]`  (no arg = full corpus)
use anyhow::Result;
use ep_rag_fetch::fetch_corpus;

fn main() -> Result<()> {
    let cap = std::env::args().nth(1).and_then(|s| s.parse::<usize>().ok());
    println!("fetch_corpus(EMPL/REGI/IMCO, 2024, per_type_limit={cap:?})");
    fetch_corpus(&["EMPL", "REGI", "IMCO"], 2024, cap)?;
    Ok(())
}
