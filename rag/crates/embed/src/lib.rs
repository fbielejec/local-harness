//! BGE embeddings in PURE RUST via `candle` (no ONNX Runtime, no C++).
//!
//! We pivoted here from `fastembed` because its ONNX Runtime prebuilt requires
//! glibc >= 2.38, but this box is glibc 2.35 (Ubuntu 22.04). candle is pure Rust,
//! links cleanly, and is a *more* maximal-Rust choice. The tradeoff — we own the
//! two contract-critical details ourselves -- is exactly what the parity gate
//! validates:
//!   * CLS pooling: take the [CLS] token (index 0) of the last hidden state.
//!     bge-small uses CLS pooling, NOT mean.
//!   * L2 normalization (BGE is cosine-trained).
//! The query-only instruction prefix (Drill-1's surface) is applied explicitly.

use anyhow::{anyhow, Result};
use candle_core::{DType, Device, IndexOp, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config};
use ep_rag_core::EmbeddingContract;
use hf_hub::{api::sync::Api, Repo, RepoType};
use tokenizers::{PaddingParams, PaddingStrategy, Tokenizer};

pub struct Embedder {
    model: BertModel,
    tokenizer: Tokenizer,
    device: Device,
    contract: EmbeddingContract,
}

impl Embedder {
    pub fn new() -> Result<Self> {
        let contract = EmbeddingContract::default();
        let device = Device::Cpu;

        // Download config + tokenizer + weights from the HF hub (cached).
        let api = Api::new()?;
        let repo = api.repo(Repo::new(contract.model.clone(), RepoType::Model));
        let config: Config =
            serde_json::from_str(&std::fs::read_to_string(repo.get("config.json")?)?)?;
        let mut tokenizer =
            Tokenizer::from_file(repo.get("tokenizer.json")?).map_err(|e| anyhow!("{e}"))?;
        // Deterministic batching: pad to the longest sequence in the batch.
        tokenizer.with_padding(Some(PaddingParams {
            strategy: PaddingStrategy::BatchLongest,
            ..Default::default()
        }));

        let weights = repo.get("model.safetensors")?;
        let vb = unsafe { VarBuilder::from_mmaped_safetensors(&[weights], DType::F32, &device)? };
        let model = BertModel::load(vb, &config)?;

        Ok(Self {
            model,
            tokenizer,
            device,
            contract,
        })
    }

    pub fn contract(&self) -> &EmbeddingContract {
        &self.contract
    }

    /// Core: tokenize -> BERT forward -> CLS pool -> L2 normalize.
    fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        let encodings = self
            .tokenizer
            .encode_batch(texts, true)
            .map_err(|e| anyhow!("{e}"))?;
        let n = encodings.len();
        let seq = encodings[0].get_ids().len();

        let ids: Vec<u32> = encodings
            .iter()
            .flat_map(|e| e.get_ids().to_vec())
            .collect();
        let mask: Vec<u32> = encodings
            .iter()
            .flat_map(|e| e.get_attention_mask().to_vec())
            .collect();

        let input_ids = Tensor::from_vec(ids, (n, seq), &self.device)?;
        let attention = Tensor::from_vec(mask, (n, seq), &self.device)?;
        let token_type_ids = input_ids.zeros_like()?;

        // last hidden state: [n, seq, hidden]
        let hidden = self
            .model
            .forward(&input_ids, &token_type_ids, Some(&attention))?;
        // CLS pooling -> [n, hidden]
        let cls = hidden.i((.., 0))?;
        // L2 normalize per row
        let norm = cls.sqr()?.sum_keepdim(1)?.sqrt()?;
        let normalized = cls.broadcast_div(&norm)?;
        Ok(normalized.to_vec2::<f32>()?)
    }

    /// Embed passages -- NO instruction prefix.
    pub fn embed_passages(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        self.embed(texts)
    }

    /// Embed queries -- WITH the contract's instruction prefix prepended.
    pub fn embed_queries(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        let prefixed = texts
            .into_iter()
            .map(|t| format!("{} {}", self.contract.query_instruction, t))
            .collect();
        self.embed(prefixed)
    }
}
