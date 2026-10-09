//! Kiwi adapter.
//!
//! `kiwi-rs` loads Kiwi's C library at runtime and is not documented as
//! `Send + Sync`, so the instance lives on one dedicated thread and requests
//! travel over a channel. A call costs microseconds of channel overhead on top
//! of Kiwi's own sub-millisecond analysis, and no `unsafe` is needed.

use std::sync::mpsc::{self, Sender};
use std::sync::Mutex;
use std::thread;

use kiwi_rs::Kiwi;

use crate::morph::{MorphError, Morphology, RawToken};

type Reply = mpsc::Sender<Result<Vec<RawToken>, MorphError>>;

enum Request {
    Tokenize(String, Reply),
}

pub struct KiwiAnalyzer {
    // `Sender` is Send but not Sync on older toolchains; the mutex makes the
    // adapter Sync without any unsafe code and is uncontended in practice.
    tx: Mutex<Sender<Request>>,
}

impl KiwiAnalyzer {
    /// Load Kiwi using `KIWI_LIBRARY_PATH`/`KIWI_MODEL_PATH` or the bootstrap
    /// cache; downloads the matching release on first use (see `Kiwi::init`).
    pub fn spawn() -> Result<Self, MorphError> {
        Self::spawn_with(|| Kiwi::init().map_err(|e| e.to_string()))
    }

    /// Load Kiwi from explicit paths (no network).
    pub fn spawn_from_paths(library: std::path::PathBuf, model: std::path::PathBuf) -> Result<Self, MorphError> {
        Self::spawn_with(move || {
            let cfg = kiwi_rs::KiwiConfig::default().with_library_path(library).with_model_path(model);
            Kiwi::from_config(cfg).map_err(|e| e.to_string())
        })
    }

    fn spawn_with(init: impl FnOnce() -> Result<Kiwi, String> + Send + 'static) -> Result<Self, MorphError> {
        let (tx, rx) = mpsc::channel::<Request>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        thread::Builder::new()
            .name("kiwi".into())
            .spawn(move || {
                let kiwi = match init() {
                    Ok(k) => {
                        let _ = ready_tx.send(Ok(()));
                        k
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                while let Ok(Request::Tokenize(text, reply)) = rx.recv() {
                    let result = kiwi
                        .tokenize(&text)
                        .map(|tokens| {
                            tokens
                                .into_iter()
                                .map(|t| RawToken::new(t.form, t.tag, t.position, t.position + t.length.max(1)))
                                .collect()
                        })
                        .map_err(|e| MorphError::Failed(e.to_string()));
                    let _ = reply.send(result);
                }
            })
            .map_err(|e| MorphError::Unavailable(e.to_string()))?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self { tx: Mutex::new(tx) }),
            Ok(Err(e)) => Err(MorphError::Unavailable(e)),
            Err(_) => Err(MorphError::Unavailable("kiwi thread exited during start-up".into())),
        }
    }
}

impl Morphology for KiwiAnalyzer {
    fn name(&self) -> &'static str {
        "kiwi"
    }

    fn tokenize(&self, text: &str) -> Result<Vec<RawToken>, MorphError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx
            .lock()
            .map_err(|_| MorphError::Failed("kiwi channel poisoned".into()))?
            .send(Request::Tokenize(text.to_owned(), reply_tx))
            .map_err(|_| MorphError::Unavailable("kiwi thread is gone".into()))?;
        reply_rx.recv().map_err(|_| MorphError::Failed("kiwi thread dropped the request".into()))?
    }
}
