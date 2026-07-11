//! Grounded-prompt assembly + `<think>` stripping.

use ep_rag_retrieve::Hit;

/// The grounding discipline — copied verbatim from rag_query.org block B.
pub const SYSTEM: &str = "You are a research assistant for European Parliament committee documents. \
Answer the question USING ONLY the numbered context passages below. \
After each claim, cite the passage id in square brackets, e.g. [EMPL-PR-785214:3]. \
If the answer is not contained in the context, reply with exactly: I don't know.";

/// Concatenate top-k as `[cid]\ntext` (equal concat = the industry mean-pool) and
/// wrap with the question. Returns `(system, user)`.
pub fn assemble(question: &str, hits: &[Hit]) -> (String, String) {
    let context = hits
        .iter()
        .map(|h| format!("[{}]\n{}", h.citation_id, h.text))
        .collect::<Vec<_>>()
        .join("\n\n");
    let user = format!("Context:\n\n{context}\n\n---\nQuestion: {question}");
    (SYSTEM.to_string(), user)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ep_rag_retrieve::Hit;

    fn hits() -> Vec<Hit> {
        vec![
            Hit { citation_id: "EMPL-PR-785214:1".into(), doc_id: "EMPL-PR-785214".into(),
                  score: 0.73, text: "young person under the age of 30 ... within four months".into(),
                  title: "Youth Guarantee".into() },
            Hit { citation_id: "EMPL-PR-785214:3".into(), doc_id: "EMPL-PR-785214".into(),
                  score: 0.69, text: "The evaluation of the reinforced Youth Guarantee".into(),
                  title: "Youth Guarantee".into() },
        ]
    }

    #[test]
    fn assemble_fixes_grounding_and_labels_context() {
        let (system, user) = assemble("What is the deadline?", &hits());
        assert!(system.contains("USING ONLY"));
        assert!(system.contains("I don't know"));
        assert!(system.contains("square brackets"));
        assert!(user.contains("[EMPL-PR-785214:1]"));
        assert!(user.contains("within four months"));
        assert!(user.contains("Question: What is the deadline?"));
    }

    #[test]
    fn assemble_with_no_hits_still_produces_a_prompt() {
        let (_system, user) = assemble("anything", &[]);
        assert!(user.contains("Question: anything"));
    }
}
