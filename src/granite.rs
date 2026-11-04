//! Granite Embedding 311M multilingual R2 (IBM, Apache-2.0) through ONNX Runtime: the one `setup
//! model` leaves beside the models (`onnxruntime/`), else `ORT_DYLIB_PATH`, else the system's. With
//! a runtime that has CUDA it runs on the GPU, otherwise (or when the GPU fails) on the CPU, always with
//! the full model (`model.onnx`): on the CPU it takes ~26 ms per memory and ranks better than the
//! quantized one (90 % against 85 % first hits), which is only used when it is the only file. The
//! vector is the CLS token's, its first 384 components (Matryoshka), L2-normalized: the 384 of
//! 768 that rank Spanish memories as well as the whole (`bench/embeddings/results.md`).
//!
//! The model leaves memory (and the GPU) after five idle minutes, like the static model.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use ort::environment::Environment;
use ort::ep::{self, ExecutionProvider};
use ort::session::Session;
use ort::value::Tensor;
use tokenizers::{PaddingParams, Tokenizer, TruncationParams};

use crate::embed::{Embedder, catch_panic};
use crate::error::{Error, Result};
use crate::idle::{DEFAULT_IDLE, IdleCache};

/// What a vector is called in the file: the model and the cut.
pub const GRANITE_ID: &str = "granite-embedding-311m-multilingual-r2@384";
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

/// The runtime has CUDA (whether the GPU takes a session is only known when it does). The CUDA and
/// cuDNN libraries `setup model` brought are loaded first, so the provider finds them; the
/// system's are found by name.
fn cuda(runtime: &Path) -> bool {
    if !ep::CUDA::is_available().unwrap_or(false) {
        return false;
    }
    let libs: Vec<PathBuf> = std::fs::read_dir(runtime)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path().join("lib"))
        .collect();
    for name in ep::cuda::CUDA_DYLIBS.iter().chain(ep::cuda::CUDNN_DYLIBS) {
        if let Some(p) = libs.iter().map(|d| d.join(name)).find(|p| p.is_file()) {
            // A library that does not load leaves the provider to fail, and the CPU to run.
            let _ = ort::util::preload_dylib(p);
        }
    }
    true
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
    device: &'static str,
}

impl Granite {
    /// From `dir` (`tokenizer.json`, and `model.onnx` and/or `model_quint8_avx2.onnx`): the full
    /// model when it is there, on the GPU when the runtime has CUDA, else on the CPU.
    pub fn load(dir: &Path) -> Result<Self> {
        let file = [FULL, QUANTIZED]
            .into_iter()
            .find(|f| dir.join(f).is_file())
            .ok_or_else(|| err(format!("no {FULL} or {QUANTIZED} in {}", dir.display())))?;
        Self::load_file(dir, file)
    }

    /// From `dir`, with this model file (the GPU only for the full model).
    pub fn load_file(dir: &Path, file: &str) -> Result<Self> {
        if !dir.join("tokenizer.json").is_file() {
            return Err(err(format!("no tokenizer.json in {}", dir.display())));
        }
        let runtime = runtime_of(dir);
        env(&runtime)?;
        let gpu = file == FULL && cuda(&runtime);
        let (dir, file) = (dir.to_path_buf(), file.to_owned());
        let load = move || read(&dir, &file, gpu);
        let first = load()?;
        let device = if first.1 { "cuda" } else { "cpu" };
        // Said once, here: the GPU was there and did not take the model.
        if gpu && !first.1 {
            eprintln!(
                "clio-memory: la GPU no respondió; el modelo de vectores corre en el procesador"
            );
        }
        Ok(Self {
            model: IdleCache::new(first.0, Some(DEFAULT_IDLE), move || load().map(|l| l.0)),
            device,
        })
    }

    /// Where it runs: `"cuda"` or `"cpu"`.
    pub fn device(&self) -> &'static str {
        self.device
    }
}

/// The session and the tokenizer, and whether the GPU took the session.
fn read(dir: &Path, file: &str, gpu: bool) -> Result<(Loaded, bool)> {
    let mut tokenizer = Tokenizer::from_file(dir.join("tokenizer.json")).map_err(err)?;
    tokenizer
        .with_truncation(Some(TruncationParams {
            max_length: MAX_TOKENS,
            ..TruncationParams::default()
        }))
        .map_err(err)?;
    tokenizer.with_padding(Some(PaddingParams::default()));
    let path = dir.join(file);
    let open = |cuda: bool| -> Result<Session> {
        let builder = Session::builder(env(&runtime_of(dir))?).map_err(err)?;
        let mut builder = if cuda {
            builder
                .with_execution_providers([ep::CUDA::default().build().error_on_failure()])
                .map_err(err)?
        } else {
            builder
        };
        builder.commit_from_file(&path).map_err(err)
    };
    if gpu && let Ok(session) = open(true) {
        return Ok((Loaded { session, tokenizer }, true));
    }
    let session = open(false)?;
    Ok((Loaded { session, tokenizer }, false))
}

impl Embedder for Granite {
    fn model_id(&self) -> &str {
        GRANITE_ID
    }

    fn dim(&self) -> usize {
        DIM
    }

    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        self.model
            .with(|m| catch_panic(|| run(m, texts)))?
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
