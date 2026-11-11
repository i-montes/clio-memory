//! Granite Embedding 311M multilingual R2 (IBM, Apache-2.0) through ONNX Runtime, on the CPU: the
//! runtime `setup model` leaves beside the models (`onnxruntime/`), else `ORT_DYLIB_PATH`, else the
//! system's. The quantized model (`model_quint8_avx2.onnx`, 313 MB) when it is there: about 490 MB
//! of memory and 26 ms per memory, with the same first five as the full one (97.5 %; 85 % of first
//! hits against 90 %). The full model (`model.onnx`, 1.2 GB, about 2 GB of memory) only when it is
//! the only file; its vectors are another space (`GRANITE_ID` and `GRANITE_Q8_ID`), so a switch
//! computes them again. The vector is the CLS token's, its first 384 components (Matryoshka),
//! L2-normalized: the 384 of 768 that rank Spanish memories as well as the whole
//! (`bench/embeddings/results.md`).
//!
//! The model is read when there is a text to turn into a vector, not when the memory opens (most
//! calls need none), and leaves memory after a minute without use.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use ort::environment::Environment;
use ort::session::Session;
use ort::value::Tensor;
use tokenizers::{PaddingParams, Tokenizer, TruncationParams};

use crate::embed::{Embedder, catch_panic};
use crate::error::{Error, Result};
use crate::idle::{DEFAULT_IDLE, IdleCache};

pub use crate::embed::{GRANITE_ID, GRANITE_Q8_ID};
const DIM: usize = 384;
/// Tokens per text: a memory is at most 1,200 characters, well under this.
const MAX_TOKENS: usize = 1024;
const FULL: &str = "model.onnx";
const QUANTIZED: &str = "model_quint8_avx2.onnx";

fn err(e: impl std::fmt::Display) -> Error {
    Error::Embedding(format!("granite: {e}"))
}

/// The ONNX Runtime library, loaded once per process. A missing library is an error here, never a
/// panic later.
static ENV: OnceLock<std::result::Result<Environment, String>> = OnceLock::new();

fn dylib() -> String {
    format!(
        "{}onnxruntime{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    )
}

/// `ORT_DYLIB_PATH`, else the runtime `setup model` installed in `runtime`, else the system's.
fn runtime_lib(runtime: &Path) -> PathBuf {
    std::env::var_os("ORT_DYLIB_PATH")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .or_else(|| Some(runtime.join("ort/lib").join(dylib())).filter(|p| p.is_file()))
        .unwrap_or_else(|| PathBuf::from(dylib()))
}

fn env(runtime: &Path) -> Result<&'static Environment> {
    ENV.get_or_init(|| {
        let path = runtime_lib(runtime);
        ort::init_from(&path)
            .map_err(|e| format!("no se pudo cargar ONNX Runtime ({}): {e}", path.display()))?
            .build()
            .map_err(|e| e.to_string())
    })
    .as_ref()
    .map_err(err)
}

/// Where `setup model` leaves the runtime: beside the model's folder.
fn runtime_of(dir: &Path) -> PathBuf {
    dir.parent().unwrap_or(dir).join("onnxruntime")
}

struct Loaded {
    session: Session,
    tokenizer: Tokenizer,
}

pub struct Granite {
    model: IdleCache<Loaded>,
    id: &'static str,
}

impl Granite {
    /// From `dir` (`tokenizer.json`, and `model_quint8_avx2.onnx` and/or `model.onnx`): the
    /// quantized model when it is there, else the full one. Checks the files and loads the runtime
    /// library; the model itself is read on the first text.
    pub fn load(dir: &Path) -> Result<Self> {
        let file = [QUANTIZED, FULL]
            .into_iter()
            .find(|f| dir.join(f).is_file())
            .ok_or_else(|| err(format!("no {QUANTIZED} or {FULL} in {}", dir.display())))?;
        Self::load_file(dir, file)
    }

    /// From `dir`, with this model file (`model_quint8_avx2.onnx` or `model.onnx`).
    pub fn load_file(dir: &Path, file: &str) -> Result<Self> {
        let id = match file {
            QUANTIZED => GRANITE_Q8_ID,
            FULL => GRANITE_ID,
            other => return Err(err(format!("{other} is not a Granite model file"))),
        };
        for f in ["tokenizer.json", file] {
            if !dir.join(f).is_file() {
                return Err(err(format!("no {f} in {}", dir.display())));
            }
        }
        if !looks_like_onnx(&dir.join(file)) {
            return Err(err(format!(
                "{file} in {} is not an ONNX model",
                dir.display()
            )));
        }
        // A runtime that is missing or does not load is said now, while the caller can still take
        // another model; it is loaded once per process and is small beside the model.
        env(&runtime_of(dir))?;
        let (dir, file) = (dir.to_path_buf(), file.to_owned());
        Ok(Self {
            model: IdleCache::empty(DEFAULT_IDLE, move || read(&dir, &file)),
            id,
        })
    }
}

/// Whether `path` can be an ONNX model: at least a megabyte, starting with its IR version (field 1,
/// a varint). The model is only read on the first text, so a file cut short or of something else is
/// caught here, where the caller can still take another model.
fn looks_like_onnx(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let big = f.metadata().is_ok_and(|m| m.len() >= 1 << 20);
    let mut first = [0u8; 1];
    big && f.read_exact(&mut first).is_ok() && first[0] == 0x08
}

/// The session (on the CPU) and the tokenizer.
fn read(dir: &Path, file: &str) -> Result<Loaded> {
    let mut tokenizer = Tokenizer::from_file(dir.join("tokenizer.json")).map_err(err)?;
    tokenizer
        .with_truncation(Some(TruncationParams {
            max_length: MAX_TOKENS,
            ..TruncationParams::default()
        }))
        .map_err(err)?;
    tokenizer.with_padding(Some(PaddingParams::default()));
    let session = Session::builder(env(&runtime_of(dir))?)
        .map_err(err)?
        .commit_from_file(dir.join(file))
        .map_err(err)?;
    Ok(Loaded { session, tokenizer })
}

impl Embedder for Granite {
    fn model_id(&self) -> &str {
        self.id
    }

    fn dim(&self) -> usize {
        DIM
    }

    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        // One text at a time: on the CPU a batch is padded to its longest text, so it is slower
        // (68 memories: 5.8 s in batches of 32, 2.1 s one by one) and holds more memory at once.
        self.model
            .with(|m| {
                texts
                    .iter()
                    .map(|t| catch_panic(|| run(m, &[t])).and_then(|r| r))
                    .map(|r| r.and_then(|mut v| v.pop().ok_or_else(|| err("no vector"))))
                    .collect::<Result<Vec<_>>>()
            })
            .and_then(|r| r)
    }
}

fn run(m: &mut Loaded, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
    let enc = m
        .tokenizer
        .encode_batch(texts.to_vec(), true)
        .map_err(err)?;
    let (batch, len) = (enc.len(), enc.first().map_or(0, |e| e.len()));
    let mut ids = Vec::with_capacity(batch * len);
    let mut mask = Vec::with_capacity(batch * len);
    for e in &enc {
        ids.extend(e.get_ids().iter().map(|&x| i64::from(x)));
        mask.extend(e.get_attention_mask().iter().map(|&x| i64::from(x)));
    }
    let shape = [batch, len];
    let ids = Tensor::from_array((shape, ids)).map_err(err)?;
    let mask = Tensor::from_array((shape, mask)).map_err(err)?;
    let out = m
        .session
        .run(ort::inputs!["input_ids" => ids, "attention_mask" => mask])
        .map_err(err)?;
    let (dims, hidden) = out["last_hidden_state"]
        .try_extract_tensor::<f32>()
        .map_err(err)?;
    let width = usize::try_from(*dims.last().ok_or_else(|| err("empty output"))?).map_err(err)?;
    if width < DIM {
        return Err(Error::BadVector);
    }
    let per_text = len * width;
    Ok((0..batch)
        .map(|b| {
            // The CLS token is the first of each text; its first 384 components, normalized.
            let cls = &hidden[b * per_text..b * per_text + DIM];
            let norm = cls
                .iter()
                .map(|x| x * x)
                .sum::<f32>()
                .sqrt()
                .max(f32::MIN_POSITIVE);
            cls.iter().map(|x| x / norm).collect()
        })
        .collect())
}
