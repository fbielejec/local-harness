//! rag-server config. Env-driven so laptop<->weebeastie is a config swap.

/// rag-server config. Env-driven so laptop<->weebeastie is a config swap.
#[derive(Debug, Clone)]
pub struct Config {
    pub qdrant_url: String,
    pub gen_base_url: String,
    pub bind_addr: String,
    pub manifest_path: String,
    pub top_k: u64,
    pub model_id: String,   // the name OWUI shows in its model picker
    pub strip_think: bool,
}

impl Config {
    /// Testable core: reads via a closure so tests inject env without touching the process.
    pub fn from_reader(get: impl Fn(&str) -> Option<String>) -> Self {
        let s = |k: &str, d: &str| get(k).unwrap_or_else(|| d.to_string());
        Config {
            qdrant_url: s("QDRANT_URL", "http://localhost:6334"),
            gen_base_url: s("GEN_BASE_URL", "http://localhost:8080/v1"),
            bind_addr: s("RAG_BIND_ADDR", "127.0.0.1:8081"),
            manifest_path: s("MANIFEST_PATH", "data/manifest.jsonl"),
            top_k: s("TOP_K", "5").parse().unwrap_or(5),
            model_id: s("RAG_MODEL_ID", "ep-committees-grounded"),
            strip_think: get("STRIP_THINK").as_deref() == Some("1"),
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
    fn config_from_env_reader_applies_defaults_and_overrides() {
        let get = |k: &str| match k {
            "QDRANT_URL" => Some("http://x:6334".to_string()),
            "TOP_K" => Some("8".to_string()),
            _ => None,
        };
        let c = Config::from_reader(get);
        assert_eq!(c.qdrant_url, "http://x:6334");
        assert_eq!(c.top_k, 8);
        assert_eq!(c.gen_base_url, "http://localhost:8080/v1");
        assert_eq!(c.bind_addr, "127.0.0.1:8081");
        assert_eq!(c.manifest_path, "data/manifest.jsonl");
        assert_eq!(c.model_id, "ep-committees-grounded");
    }
}
