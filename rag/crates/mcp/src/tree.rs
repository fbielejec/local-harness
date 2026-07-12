use anyhow::Result;

/// The routing tree, loaded as opaque JSON (we only need version + the raw object to inline
/// into the classify prompt; the model walks the structure, not Rust).
pub struct Tree {
    pub version: String,
    raw: serde_json::Value,
}

impl Tree {
    pub fn from_json(s: &str) -> Result<Self> {
        let raw: serde_json::Value = serde_json::from_str(s)?;
        let version = raw.get("version").and_then(|v| v.as_str()).unwrap_or("?").to_string();
        Ok(Self { version, raw })
    }

    pub fn load(path: &str) -> Result<Self> {
        Self::from_json(&std::fs::read_to_string(path)?)
    }

    /// Build the Mode-A classify prompt: tree in the system prompt, JSON-only output contract.
    /// Returns `(system, user)`. Mirrors scratchpad/tree_spike.py::SYS_A (proven 10/10).
    pub fn classify_prompt(&self, message: &str) -> (String, String) {
        let tree = serde_json::to_string_pretty(&self.raw).unwrap_or_default();
        let system = format!(
            "You are a task router. Below is a decision tree as JSON. Walk it starting at `root`. \
For the user's message, answer the `question` node(s) and follow yes/no until you reach a `result` node.\n\n\
DECISION TREE:\n{tree}\n\n\
Respond with ONLY a compact JSON object, no prose, no markdown fence:\n\
{{\"reached\": \"<result node id>\", \"tool\": \"<tool name or null>\", \"reason\": \"<one short clause>\"}}"
        );
        (system, message.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TREE: &str = r#"{"version":"1.0","root":"Q","nodes":{"Q":{"type":"question","question":"about EP committees?","help":"YES: x NO: y","yes":"R_USE_EP_RAG","no":"R_UNCLASSIFIED"}}}"#;

    #[test]
    fn loads_tree_from_json() {
        let t = Tree::from_json(TREE).unwrap();
        assert_eq!(t.version, "1.0");
    }

    #[test]
    fn classify_prompt_embeds_tree_and_demands_json() {
        let t = Tree::from_json(TREE).unwrap();
        let (system, user) = t.classify_prompt("What is the Youth Guarantee deadline?");
        assert!(system.contains("about EP committees?"));       // the tree is in the prompt
        assert!(system.contains("\"reached\""));                 // output contract
        assert!(system.contains("R_USE_EP_RAG"));
        assert!(user.contains("Youth Guarantee"));               // the message is the user turn
    }
}
