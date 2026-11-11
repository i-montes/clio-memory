//! ms per memory with Granite on this machine: `CLIO_MEMORY_GRANITE_DIR=… cargo run --release
//! --features granite --example granite_time [model_quint8_avx2.onnx|model.onnx]`.
use std::time::Instant;

use clio_memory::{Embedder, Granite};

fn main() {
    let dir = std::path::PathBuf::from(
        std::env::var("CLIO_MEMORY_GRANITE_DIR").expect("CLIO_MEMORY_GRANITE_DIR"),
    );
    let file = std::env::args().nth(1);
    let t = Instant::now();
    let g = match &file {
        Some(f) => Granite::load_file(&dir, f),
        None => Granite::load(&dir),
    }
    .expect("load");
    let load = t.elapsed();
    let text = "Error: el contenedor de la API arrancaba antes que la base y fallaba · Causa: compose no espera sin healthcheck · Arreglo: healthcheck y depends_on";
    // The model is read on the first text.
    let t = Instant::now();
    g.embed(&[text]).expect("warm up");
    let first = t.elapsed();
    let mut ms: Vec<f64> = (0..30)
        .map(|_| {
            let t = Instant::now();
            g.embed(&[text]).expect("embed");
            t.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    ms.sort_by(f64::total_cmp);
    println!(
        "{} ({}): ready {:.1} s, first text {:.1} s, median {:.1} ms per memory",
        file.as_deref().unwrap_or("auto"),
        g.model_id(),
        load.as_secs_f64(),
        first.as_secs_f64(),
        ms[ms.len() / 2]
    );
}
