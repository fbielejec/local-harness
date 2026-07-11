//! Fetch EP committee documents from the Open Data Portal (ODP).
//!
//! Rust port of the verified Python recon. Three stages (all confirmed live):
//!   1. LIST     `/committee-documents?year=Y&limit=N`  (slow; cached to disk).
//!              No server-side committee filter → scope client-side by identifier
//!              prefix + work_type.
//!   2. RESOLVE  `/documents/{ID}?language=en` → walk is_realized_by →
//!              is_embodied_by, pick the /pdf manifestation; its
//!              is_exemplified_by[0] is a HOST-RELATIVE distribution path.
//!   3. DOWNLOAD prefix the host; reqwest follows the 302 to the file store.
use anyhow::{anyhow, Result};
use reqwest::blocking::Client;
use reqwest::header::ACCEPT;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

const API: &str = "https://data.europarl.europa.eu/api/v2";
const HOST: &str = "https://data.europarl.europa.eu";

/// work_type (last URI segment) -> our short doc_type code.
pub fn doc_types() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        ("REPORT_PARLIAMENTARY_COMMITTEE_DRAFT", "PR"), // draft report
        ("OPINION_PARLIAMENTARY_COMMITTEE", "AD"),      // adopted opinion
    ])
}

#[derive(Debug, Clone)]
pub struct DocRef {
    pub doc_id: String,
    pub committee: String,
    pub doc_type: String,
}

/// A target with its EN PDF url + metadata, written to the manifest after download.
#[derive(Debug, Clone, Serialize)]
pub struct FetchRecord {
    pub doc_id: String,
    pub committee: String,
    pub doc_type: String,
    pub title: String,
    pub pdf_url: String,
    pub issued: String,
    pub byte_size: u64,
    pub pdf_path: String,
}

fn client() -> Result<Client> {
    Ok(Client::builder()
        .user_agent("ep-rag/0.1 (research)")
        .timeout(Duration::from_secs(180))
        .build()?)
}

/// GET as JSON-LD with a small retry (the list endpoint flaked once). Returns the
/// status and body so callers can special-case 404 (missing document).
fn get_ldjson(client: &Client, url: &str, query: &[(&str, String)]) -> Result<(u16, String)> {
    let mut last = anyhow!("no attempt");
    for attempt in 0..4u32 {
        match client
            .get(url)
            .header(ACCEPT, "application/ld+json")
            .query(query)
            .send()
        {
            Ok(resp) => {
                let status = resp.status();
                if status.is_server_error() || status.as_u16() == 429 {
                    last = anyhow!("status {status}");
                } else {
                    return Ok((status.as_u16(), resp.text()?));
                }
            }
            Err(e) => last = anyhow!(e),
        }
        std::thread::sleep(Duration::from_secs(1u64 << attempt)); // 1,2,4,8s
    }
    Err(last)
}

/// Stage 1: the (slow) yearly list, cached to disk so we hit ODP only once.
pub fn list_committee_documents(year: u32) -> Result<Vec<Value>> {
    let cache = PathBuf::from(format!("data/raw/committee-documents-{year}.json"));
    if cache.exists() {
        let v: Value = serde_json::from_str(&fs::read_to_string(&cache)?)?;
        return Ok(v["data"].as_array().cloned().unwrap_or_default());
    }
    let (_s, body) = get_ldjson(
        &client()?,
        &format!("{API}/committee-documents"),
        &[("year", year.to_string()), ("limit", "4000".into())],
    )?;
    fs::create_dir_all(cache.parent().unwrap())?;
    fs::write(&cache, &body)?;
    let v: Value = serde_json::from_str(&body)?;
    Ok(v["data"].as_array().cloned().unwrap_or_default())
}

/// Client-side filter: identifier prefix + work_type (no server filter exists).
pub fn select(items: &[Value], committees: &[&str]) -> Vec<DocRef> {
    let dt = doc_types();
    let mut out = Vec::new();
    for it in items {
        let ident = it["identifier"].as_str().unwrap_or("");
        let wt = it["work_type"].as_str().unwrap_or("").rsplit('/').next().unwrap_or("");
        let committee = ident.split('-').next().unwrap_or("");
        if committees.contains(&committee) {
            if let Some(code) = dt.get(wt) {
                out.push(DocRef {
                    doc_id: ident.to_string(),
                    committee: committee.to_string(),
                    doc_type: (*code).to_string(),
                });
            }
        }
    }
    out
}

/// Stage 2: detail record -> EN PDF url + title + metadata. None if no EN PDF.
pub fn resolve(client: &Client, r: &DocRef) -> Result<Option<FetchRecord>> {
    let (status, body) = get_ldjson(
        client,
        &format!("{API}/documents/{}", r.doc_id),
        &[("language", "en".into())],
    )?;
    if status == 404 {
        return Ok(None);
    }
    let v: Value = serde_json::from_str(&body)?;
    let work = &v["data"][0];
    let title = work["title_dcterms"]["en"].as_str().unwrap_or("").to_string();
    for expr in work["is_realized_by"].as_array().into_iter().flatten() {
        for man in expr["is_embodied_by"].as_array().into_iter().flatten() {
            let id = man["id"].as_str().unwrap_or("");
            let mt = man["media_type"].as_str().unwrap_or("");
            if !(id.ends_with("/pdf") || mt.contains("application/pdf")) {
                continue;
            }
            // is_exemplified_by is a list of host-relative distribution paths.
            let rel = match &man["is_exemplified_by"] {
                Value::Array(a) => a.first().and_then(|x| x.as_str()),
                Value::String(s) => Some(s.as_str()),
                _ => None,
            };
            let Some(rel) = rel else { continue };
            return Ok(Some(FetchRecord {
                doc_id: r.doc_id.clone(),
                committee: r.committee.clone(),
                doc_type: r.doc_type.clone(),
                title,
                pdf_url: format!("{HOST}/{rel}"),
                issued: man["issued"].as_str().unwrap_or("").to_string(),
                byte_size: 0, // filled from the real file after download
                pdf_path: String::new(),
            }));
        }
    }
    Ok(None)
}

/// Stage 3: download the PDF (reqwest follows the 302), guard the magic bytes.
pub fn download(client: &Client, rec: &mut FetchRecord, skip_existing: bool) -> Result<()> {
    let dest = PathBuf::from(format!("data/pdfs/{}/{}_en.pdf", rec.committee, rec.doc_id));
    if !(skip_existing && dest.exists() && fs::metadata(&dest)?.len() > 0) {
        let bytes = client.get(&rec.pdf_url).send()?.error_for_status()?.bytes()?;
        if !bytes.starts_with(b"%PDF-") {
            return Err(anyhow!("not a PDF ({} bytes)", bytes.len()));
        }
        fs::create_dir_all(dest.parent().unwrap())?;
        fs::write(&dest, &bytes)?;
    }
    rec.byte_size = fs::metadata(&dest)?.len(); // real on-disk size (ODP byteSize is unreliable)
    rec.pdf_path = dest.to_string_lossy().into_owned();
    Ok(())
}

/// Resolve + download every target; write data/manifest.jsonl (handoff to parse).
/// One bad document can't abort the corpus build.
pub fn fetch_corpus(committees: &[&str], year: u32, per_type_limit: Option<usize>) -> Result<usize> {
    let items = list_committee_documents(year)?;
    let mut refs = select(&items, committees);
    if let Some(cap) = per_type_limit {
        let mut counts: HashMap<(String, String), usize> = HashMap::new();
        refs.retain(|r| {
            let c = counts.entry((r.committee.clone(), r.doc_type.clone())).or_insert(0);
            (*c < cap).then(|| *c += 1).is_some()
        });
    }

    let client = client()?;
    let mut records: Vec<FetchRecord> = Vec::new();
    let mut failed: Vec<(String, String)> = Vec::new();
    let total = refs.len();
    for (i, r) in refs.iter().enumerate() {
        match resolve(&client, r) {
            Ok(Some(mut rec)) => match download(&client, &mut rec, true) {
                Ok(()) => {
                    println!("  [{}/{total}] {} -> {} ({} KB)", i + 1, r.doc_id,
                             rec.pdf_path.rsplit('/').next().unwrap_or(""), rec.byte_size / 1024);
                    records.push(rec);
                }
                Err(e) => failed.push((r.doc_id.clone(), e.to_string())),
            },
            Ok(None) => failed.push((r.doc_id.clone(), "no EN PDF".into())),
            Err(e) => failed.push((r.doc_id.clone(), e.to_string())),
        }
    }

    let manifest = PathBuf::from("data/manifest.jsonl");
    fs::create_dir_all(manifest.parent().unwrap())?;
    let body: String = records
        .iter()
        .map(|r| serde_json::to_string(r).map(|s| s + "\n"))
        .collect::<Result<String, _>>()?;
    fs::write(&manifest, body)?;
    println!("\nfetched {} PDFs -> {}   ({} failed)", records.len(), manifest.display(), failed.len());
    for (id, why) in &failed {
        println!("  ! {id}: {why}");
    }
    Ok(records.len())
}
