//! PROBE ONLY — measurement lane, not part of the shipping pipeline. Never merged into production.
//!
//! Runs a gate corpus (bench/quickfix_gate.jsonl) through either
//!   * `gector` — the shipped Quick Fix path (`Gector` + `QuickFixEngine::fix`), or
//!   * `t5`     — a T5 GEC ONNX export (encoder + merged decoder, greedy, past_key_values),
//! and writes `{"id","output","edits":[{"start","length","replacement"}],"ms"}` per line for
//! `bench/score_quickfix.py`. Edits for T5 are a word-level LCS diff of input vs output so the
//! scorer's protected-token check (G5) can see where the model touched the text.
//!
//! Usage: t5_gec_probe <t5|gector> <model_dir> <gate.jsonl> <out.jsonl> [--dump]
//!   t5:     model_dir holds onnx/encoder_model_int8.onnx, onnx/decoder_model_merged_int8.onnx,
//!           tokenizer.json, config.json. ONNX Runtime: $KEYREED_ORT_DLL or the shipped dll path.
//!   gector: model_dir holds model.onnx + tokenizer.json + config.json + guards.
//!
//! shortcut: the T5 tokenizer here is greedy longest-match over the Unigram vocab with the `▁`
//! metaspace prefix and no NFKC/precompiled-charsmap pass; that is exact for ASCII English
//! (every gate sentence), and would need the real Unigram Viterbi + normalizer for other scripts.

use anyhow::{anyhow, bail, Context, Result};
use ort::{session::{Session, SessionInputValue}, value::Tensor};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;
use vi_core::{AppCategory, WritingMode};
use vi_rewrite::gector::Gector;
use vi_rewrite::quickfix::{Edit, QuickFixEngine};

const DEFAULT_DLL: &str = "C:/Users/ImMandatory/keyreed-2.1/dist/keyreed-win-x64-slim/runtime/sherpa-onnx/onnxruntime.dll";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 4 {
        bail!("usage: t5_gec_probe <t5|gector> <model_dir> <gate.jsonl> <out.jsonl> [--dump]");
    }
    let engine = args[0].as_str();
    let dir = PathBuf::from(&args[1]);
    let gate = read_gate(&args[2])?;
    let dump = args.iter().any(|a| a == "--dump");
    let dll = std::env::var("KEYREED_ORT_DLL").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(DEFAULT_DLL));

    let mut rows = Vec::with_capacity(gate.len());
    match engine {
        "t5" => {
            let mut t5 = T5::load(&dir, &dll, dump, args.iter().any(|a| a == "--nopast"))?;
            // warm-up: two passes over the first sentence so graph init and arena growth stay out of p50/p95
            for _ in 0..2 {
                t5.generate(&gate[0].input)?;
            }
            for c in &gate {
                let t = Instant::now();
                let mut out = t5.generate(&c.input)?;
                let ms = t.elapsed().as_secs_f64() * 1000.0;
                if out.is_empty() {
                    eprintln!("t5: no output for {} (left unchanged)", c.id);
                    out = c.input.clone();
                }
                let edits = diff_edits(&c.input, &out);
                rows.push(Row { id: c.id.clone(), output: out, edits, ms });
            }
        }
        "gector" => {
            let mut qf = QuickFixEngine::new(Box::new(Gector::load(&dll, &dir, 2)?));
            for _ in 0..2 {
                qf.fix(&gate[0].input, AppCategory::Email)?;
            }
            for c in &gate {
                let cat = category(&c.context);
                let t = Instant::now();
                let fixed = qf.fix(&c.input, cat)?;
                let ms = t.elapsed().as_secs_f64() * 1000.0;
                let edits = fixed.edits.iter().map(edit_json).collect();
                rows.push(Row { id: c.id.clone(), output: fixed.corrected, edits, ms });
            }
        }
        "qwen" => {
            // the app's own local-SLM fallback, same supervisor and prompt: model_dir is the .gguf
            let server = PathBuf::from(
                std::env::var("KEYREED_LLAMA_SERVER")
                    .unwrap_or_else(|_| "C:/Users/ImMandatory/keyreed-2.1/dist/keyreed-win-x64-slim/runtime/llama/llama-server.exe".into()),
            );
            let mut sup = vi_rewrite::slm::LlamaServerSupervisor::new(server, dir.clone(), 4);
            let _ = sup.rewrite(&gate[0].input, WritingMode::Smart, category(&gate[0].context), ""); // load + warm-up
            for c in &gate {
                let cat = category(&c.context);
                let t = Instant::now();
                let out = sup.rewrite(&c.input, WritingMode::Smart, cat, "").unwrap_or_default();
                let ms = t.elapsed().as_secs_f64() * 1000.0;
                let edits = diff_edits(&c.input, &out);
                rows.push(Row { id: c.id.clone(), output: out, edits, ms });
            }
        }
        other => bail!("unknown engine {other}"),
    }
    let mut text = String::new();
    for r in &rows {
        text.push_str(&serde_json::json!({"id": r.id, "output": r.output, "edits": r.edits, "ms": r.ms}).to_string());
        text.push('\n');
    }
    std::fs::write(&args[3], text).with_context(|| format!("write {}", args[3]))?;
    let mut ms: Vec<f64> = rows.iter().map(|r| r.ms).collect();
    ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let at = |q: f64| ms[((ms.len() - 1) as f64 * q).round() as usize];
    println!("{} rows -> {}  p50 {:.1} ms  p95 {:.1} ms  max {:.1} ms", rows.len(), args[3], at(0.5), at(0.95), ms[ms.len() - 1]);
    Ok(())
}

struct Row {
    id: String,
    output: String,
    edits: Vec<serde_json::Value>,
    ms: f64,
}

fn edit_json(e: &Edit) -> serde_json::Value {
    let layer = match e.layer {
        vi_rewrite::quickfix::Layer::Spelling => "spelling",
        vi_rewrite::quickfix::Layer::Rule => "rule",
        vi_rewrite::quickfix::Layer::Grammar => "grammar",
    };
    serde_json::json!({"start": e.start, "length": e.length, "replacement": e.replacement, "layer": layer, "class": e.class})
}

fn category(context: &str) -> AppCategory {
    match context {
        "email" => AppCategory::Email,
        "chat" => AppCategory::Chat,
        "notes" => AppCategory::Notes,
        "document" => AppCategory::Document,
        "code" => AppCategory::CodeEditor,
        _ => AppCategory::General,
    }
}

struct Case {
    id: String,
    context: String,
    input: String,
}

fn read_gate(path: &str) -> Result<Vec<Case>> {
    let raw = std::fs::read_to_string(path).with_context(|| format!("read {path}"))?;
    let mut out = Vec::new();
    for line in raw.lines().filter(|l| !l.trim().is_empty()) {
        let v: serde_json::Value = serde_json::from_str(line).context("gate json")?;
        let s = |k: &str| v[k].as_str().map(str::to_string).ok_or_else(|| anyhow!("gate: no {k}"));
        out.push(Case { id: s("id")?, context: v["context"].as_str().unwrap_or("general").to_string(), input: s("input")? });
    }
    if out.is_empty() {
        bail!("{path} is empty");
    }
    Ok(out)
}

// ---------------------------------------------------------------- word diff (offsets into the input)

fn words(s: &str) -> Vec<(usize, usize, &str)> {
    let mut out = Vec::new();
    let (mut start, mut in_word) = (0, false);
    for (i, ch) in s.char_indices() {
        if ch.is_whitespace() {
            if in_word {
                out.push((start, i, &s[start..i]));
                in_word = false;
            }
        } else if !in_word {
            start = i;
            in_word = true;
        }
    }
    if in_word {
        out.push((start, s.len(), &s[start..]));
    }
    out
}

/// One edit per changed run of words; unchanged words are never inside an edit, so the scorer's
/// must_keep overlap test stays honest.
fn diff_edits(a: &str, b: &str) -> Vec<serde_json::Value> {
    let (ta, tb) = (words(a), words(b));
    let (n, m) = (ta.len(), tb.len());
    let mut lcs = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if ta[i].2 == tb[j].2 { lcs[i + 1][j + 1] + 1 } else { lcs[i + 1][j].max(lcs[i][j + 1]) };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::new();
    while i < n || j < m {
        if i < n && j < m && ta[i].2 == tb[j].2 {
            i += 1;
            j += 1;
            continue;
        }
        let (si, sj) = (i, j);
        while i < n || j < m {
            if i < n && j < m && ta[i].2 == tb[j].2 {
                break;
            }
            if j >= m || (i < n && lcs[i + 1][j] >= lcs[i][j + 1]) {
                i += 1;
            } else {
                j += 1;
            }
        }
        let start = if si < n { ta[si].0 } else { a.len() };
        let end = if i > si { ta[i - 1].1 } else { start };
        let rep = tb[sj..j].iter().map(|w| w.2).collect::<Vec<_>>().join(" ");
        out.push(serde_json::json!({"start": start, "length": end - start, "replacement": rep}));
    }
    out
}

// ---------------------------------------------------------------- T5

struct T5 {
    enc: Session,
    dec: Session,
    dec2: Option<Session>,
    pieces: Vec<String>,
    ids: HashMap<String, u32>,
    heads: usize,
    dkv: usize,
    nopast: bool,
}

impl T5 {
    fn load(dir: &Path, dll: &Path, dump: bool, nopast: bool) -> Result<Self> {
        let env = ort::init_from(dll).with_context(|| format!("load {}", dll.display()))?.with_name("t5gec-probe").build()?;
        let threads: usize = std::env::var("KEYREED_ORT_THREADS").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
        let build = |file: &str| -> Result<Session> {
            let b = Session::builder(&env)?;
            let mut b = if threads > 0 { b.with_intra_threads(threads).map_err(|e| anyhow!("intra_threads: {e}"))? } else { b };
            Ok(b.commit_from_file(dir.join(file))?)
        };
        let enc = build("onnx/encoder_model_int8.onnx")?;
        let dec = build("onnx/decoder_model_int8.onnx")?;
        let dec2 = if nopast { None } else { Some(build("onnx/decoder_with_past_model_int8.onnx")?) };
        let config: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("config.json"))?)?;
        let heads = config["num_heads"].as_u64().unwrap_or(6) as usize;
        let dkv = config["d_kv"].as_u64().unwrap_or(64) as usize;
        let tok: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("tokenizer.json"))?)?;
        let vocab = tok["model"]["vocab"].as_array().ok_or_else(|| anyhow!("tokenizer.json: no model.vocab"))?;
        let mut pieces = Vec::with_capacity(vocab.len());
        let mut ids = HashMap::with_capacity(vocab.len());
        for (i, v) in vocab.iter().enumerate() {
            let p = v[0].as_str().ok_or_else(|| anyhow!("tokenizer.json: bad piece"))?.to_string();
            ids.insert(p.clone(), i as u32);
            pieces.push(p);
        }
        if dump {
            for (what, s) in [("encoder", &enc), ("decoder", &dec)] {
                println!("{what} inputs: {:?}", s.inputs().iter().map(|i| i.name()).collect::<Vec<_>>());
                println!("{what} outputs: {:?}", s.outputs().iter().map(|o| o.name()).collect::<Vec<_>>());
            }
        }
        Ok(Self { enc, dec, dec2, pieces, ids, heads, dkv, nopast })
    }

    fn encode(&self, text: &str) -> Vec<i64> {
        let mut out = Vec::new();
        for w in text.split_whitespace() {
            let chunk = format!("\u{2581}{w}");
            let bytes = chunk.as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                let mut hit = None;
                for e in (i + 1..=bytes.len()).rev() {
                    if e - i > 24 || !chunk.is_char_boundary(e) {
                        continue;
                    }
                    if let Some(&id) = self.ids.get(&chunk[i..e]) {
                        hit = Some((id, e));
                        break;
                    }
                }
                match hit {
                    Some((id, e)) => {
                        out.push(id as i64);
                        i = e;
                    }
                    None => {
                        out.push(2);
                        i += chunk[i..].chars().next().map(char::len_utf8).unwrap_or(1);
                    }
                }
            }
        }
        out.push(1); // </s>
        out
    }

    fn decode(&self, ids: &[i64]) -> String {
        let mut s = String::new();
        for &id in ids {
            if let Some(p) = self.pieces.get(id as usize) {
                if id < 3 || id >= 32000 {
                    break;
                }
                s.push_str(p);
            }
        }
        let s = s.replace('\u{2581}', " ");
        let s = s.trim();
        s.to_string()
    }

    fn generate(&mut self, text: &str) -> Result<String> {
        let ids = self.encode(text);
        let n = ids.len();
        let hidden: (Vec<usize>, Vec<f32>) = {
            let out = self.enc.run(vec![
                ("input_ids".to_string(), Tensor::from_array((vec![1usize, n], ids.clone()))?),
                ("attention_mask".to_string(), Tensor::from_array((vec![1usize, n], vec![1i64; n]))?),
            ])?;
            let h = out["last_hidden_state"].try_extract_tensor::<f32>()?;
            (h.0.iter().map(|&d| d as usize).collect(), h.1.to_vec())
        };
        let max_new = (n * 3 + 8).min(128);
        let mut produced: Vec<i64> = Vec::new();
        let mut prefix: Vec<i64> = vec![0]; // decoder_start_token_id
        let mut past: HashMap<String, (Vec<usize>, Vec<f32>)> = HashMap::new();

        for step in 0..max_new {
            // step 0 (and every step when there is no cache) runs the plain decoder; later steps the with-past decoder.
            let pasted = !self.nopast && step > 0 && self.dec2.is_some();
            let cur = if pasted { vec![*prefix.last().unwrap()] } else { prefix.clone() };
            let sess = if pasted { self.dec2.as_mut().unwrap() } else { &mut self.dec };
            let past_names: Vec<String> = if pasted {
                sess.inputs().iter().map(|i| i.name().to_string()).filter(|n| n.starts_with("past_key_values.")).collect()
            } else {
                Vec::new()
            };
            let len = cur.len();
            let mut inputs: Vec<(String, SessionInputValue)> = Vec::with_capacity(past_names.len() + 3);
            inputs.push(("input_ids".into(), Tensor::from_array((vec![1usize, len], cur))?.into()));
            inputs.push(("encoder_attention_mask".into(), Tensor::from_array((vec![1usize, n], vec![1i64; n]))?.into()));
            inputs.push(("encoder_hidden_states".into(), Tensor::from_array(hidden.clone())?.into()));
            for name in &past_names {
                let (shape, data) = past.get(name).ok_or_else(|| anyhow!("missing {name}; have {:?}", past.keys().collect::<Vec<_>>()))?;
                inputs.push((name.clone(), Tensor::from_array((shape.clone(), data.clone()))?.into()));
            }
            let out_names: Vec<String> = sess.outputs().iter().map(|o| o.name().to_string()).collect();
            let out = sess.run(inputs)?;
            let (shape, logits) = out["logits"].try_extract_tensor::<f32>()?;
            let vocab = *shape.last().unwrap() as usize;
            let row = &logits[logits.len() - vocab..];
            let best = row.iter().enumerate().max_by(|a, b| a.1.partial_cmp(b.1).unwrap()).map(|(i, _)| i as i64).unwrap_or(1);
            if best == 1 {
                break;
            }
            produced.push(best);
            prefix.push(best);
            // No clear: the with-past decoder only re-emits decoder.*, so the cross-attention
            // (encoder.*) tensors from step 0 stay valid for every later step.
            for name in &out_names {
                if let Some(key) = name.strip_prefix("present.") {
                    let (s, d) = out[name.as_str()].try_extract_tensor::<f32>()?;
                    past.insert(format!("past_key_values.{key}"), (s.iter().map(|&x| x as usize).collect(), d.to_vec()));
                }
            }
        }
        Ok(self.decode(&produced))
    }
}