//! Env-driven configuration for rag-mcp. `from_reader` takes a closure so the parser is
//! unit-testable without touching the real process environment; `from_env` wires it to `std::env`.

#[derive(Debug, Clone)]
pub struct Config {
    pub bind_addr: String,
    pub qdrant_url: String,
    pub gen_base_url: String,
    pub tree_path: String,
    pub manifest_path: String,
    pub top_k: u64,
}

impl Config {
    pub fn from_reader(get: impl Fn(&str) -> Option<String>) -> Self {
        let s = |k: &str, d: &str| get(k).unwrap_or_else(|| d.to_string());
        Config {
            bind_addr: s("RAG_MCP_BIND_ADDR", "127.0.0.1:8082"),
            qdrant_url: s("QDRANT_URL", "http://localhost:6334"),
            gen_base_url: s("GEN_BASE_URL", "http://localhost:8080/v1"),
            tree_path: s("TREE_PATH", "data/route_tree.json"),
            manifest_path: s("MANIFEST_PATH", "data/manifest.jsonl"),
            // Unset → silent default 5; set-but-unparseable → warn (naming the bad value)
            // then fall back, so a typo'd override never passes silently.
            top_k: match get("TOP_K") {
                None => 5,
                Some(v) => v.parse().unwrap_or_else(|_| {
                    eprintln!("warning: TOP_K={v:?} is not a valid u64; falling back to 5");
                    5
                }),
            },
        }
    }

    pub fn from_env() -> Self {
        Self::from_reader(|k| std::env::var(k).ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_and_overrides() {
        let get = |k: &str| match k {
            "TOP_K" => Some("8".into()),
            _ => None,
        };
        let c = Config::from_reader(get);
        assert_eq!(c.top_k, 8);
        assert_eq!(c.bind_addr, "127.0.0.1:8082");
        assert_eq!(c.qdrant_url, "http://localhost:6334");
        assert_eq!(c.gen_base_url, "http://localhost:8080/v1");
        assert_eq!(c.tree_path, "data/route_tree.json");
        assert_eq!(c.manifest_path, "data/manifest.jsonl");
    }
}
