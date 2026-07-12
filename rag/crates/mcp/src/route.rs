/// The routing outcome. `Unclassified` is the safe default (do not force grounding).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    UseEpRag,
    Unclassified,
}

impl Route {
    /// Parse the model's Mode-A JSON decision. Tolerates a ```json fence / surrounding prose.
    /// Any parse failure → `Unclassified` (precision-biased: never ground on uncertainty).
    pub fn from_completion(raw: &str) -> Route {
        let slice = extract_json_object(raw);
        let reached = slice
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .and_then(|v| v.get("reached").and_then(|r| r.as_str()).map(str::to_owned));
        match reached.as_deref() {
            Some("R_USE_EP_RAG") => Route::UseEpRag,
            _ => Route::Unclassified,
        }
    }

    pub fn should_ground(self) -> bool {
        matches!(self, Route::UseEpRag)
    }
}

/// Return the first `{...}` slice (outermost braces) from possibly-fenced/prose text.
/// NOTE: this assumes the outermost `{...}` span is the JSON object; a stray `{` in
/// surrounding prose yields an unparseable slice, which `from_completion` maps to the
/// safe `Unclassified` default (never grounds on uncertainty).
fn extract_json_object(raw: &str) -> Option<&str> {
    let start = raw.find('{')?;
    let end = raw.rfind('}')?;
    if end > start { Some(&raw[start..=end]) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_use_ep_rag() {
        let r = Route::from_completion(r#"{"reached":"R_USE_EP_RAG","tool":"search_ep_committee_docs","reason":"x"}"#);
        assert_eq!(r, Route::UseEpRag);
    }
    #[test]
    fn parses_unclassified() {
        let r = Route::from_completion(r#"{"reached":"R_UNCLASSIFIED","tool":null,"reason":"chit-chat"}"#);
        assert_eq!(r, Route::Unclassified);
    }
    #[test]
    fn tolerates_code_fence_and_prose() {
        let raw = "Sure:\n```json\n{\"reached\":\"R_USE_EP_RAG\",\"tool\":\"search_ep_committee_docs\"}\n```";
        assert_eq!(Route::from_completion(raw), Route::UseEpRag);
    }
    #[test]
    fn falls_back_to_unclassified_on_garbage() {
        // A safe default: if we cannot parse, do NOT force grounding (design: precision-biased).
        assert_eq!(Route::from_completion("I couldn't decide"), Route::Unclassified);
    }
    #[test]
    fn stray_brace_in_prose_falls_back_to_unclassified() {
        // A lone `{` in prose (no closing `}`, or an unparseable span) → safe default.
        assert_eq!(Route::from_completion("hmm { not json here"), Route::Unclassified);
    }
}
