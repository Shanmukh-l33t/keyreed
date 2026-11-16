//! Recognises a recording while it is still being made.
//!
//! The coordinator sends audio as it is captured; a worker thread owns the speech engine for the
//! length of the recording and decodes every finished chunk straight away. When the key is
//! released only the last chunk is left to do.

use crate::{LocalAsrEngine, TranscriptionOutput};
use anyhow::{anyhow, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Instant;
use zeroize::Zeroize;

enum Msg {
    Audio(Vec<f32>),
    /// The key was released at this instant.
    Finish(Instant),
}

/// What a finished (or cancelled) live session hands back.
pub struct LiveDone {
    /// The engine, to be reused for the next dictation. None if it could not be loaded.
    pub engine: Option<LocalAsrEngine>,
    pub result: Result<TranscriptionOutput>,
    /// Time after the release spent waiting for the model to finish loading (0 when it was warm).
    pub model_wait_ms: f64,
    /// Time from the release until the transcript was complete (includes `model_wait_ms`).
    pub post_release_ms: f64,
    /// Chunks that were already decoded when the key was released.
    pub chunks_before_release: usize,
}

impl LiveDone {
    fn failed(engine: Option<LocalAsrEngine>, err: anyhow::Error) -> Self {
        Self { engine, result: Err(err), model_wait_ms: 0.0, post_release_ms: 0.0, chunks_before_release: 0 }
    }
}

pub struct LiveAsr {
    tx: Sender<Msg>,
    released: Arc<AtomicBool>,
    worker: JoinHandle<LiveDone>,
}

impl LiveAsr {
    /// `engine` returns the loaded engine; it may block (model still loading) because it runs on
    /// the worker thread. Audio sent meanwhile simply queues up.
    pub fn start(engine: impl FnOnce() -> Result<LocalAsrEngine> + Send + 'static, hotwords: String) -> Result<Self> {
        let (tx, rx) = channel::<Msg>();
        let released = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&released);
        let worker = std::thread::Builder::new()
            .name("keyreed-live-asr".to_string())
            .spawn(move || run(engine, rx, hotwords, flag))?;
        Ok(Self { tx, released, worker })
    }

    pub fn feed(&self, samples_16k: Vec<f32>) {
        if !samples_16k.is_empty() {
            let _ = self.tx.send(Msg::Audio(samples_16k));
        }
    }

    /// The recording is over: sends the last audio and waits for the transcript.
    pub fn finish(self, last_samples_16k: Vec<f32>) -> LiveDone {
        let t_release = Instant::now();
        self.released.store(true, Ordering::Relaxed);
        self.feed(last_samples_16k);
        let _ = self.tx.send(Msg::Finish(t_release));
        self.join()
    }

    /// Throws the recording away (audio is wiped) and returns the engine.
    pub fn cancel(self) -> Option<LocalAsrEngine> {
        self.join().engine
    }

    fn join(self) -> LiveDone {
        let Self { tx, worker, .. } = self;
        drop(tx); // a closed channel without a Finish tells the worker the recording was cancelled
        worker
            .join()
            .unwrap_or_else(|_| LiveDone::failed(None, anyhow!("speech recognition worker stopped unexpectedly")))
    }
}

fn run(
    engine: impl FnOnce() -> Result<LocalAsrEngine>,
    rx: Receiver<Msg>,
    hotwords: String,
    released: Arc<AtomicBool>,
) -> LiveDone {
    let hw = Some(hotwords.as_str());
    let mut loaded = engine();
    let t_loaded = Instant::now();
    if let Ok(e) = &mut loaded {
        e.stream_begin();
    }
    let mut failure: Option<anyhow::Error> = None;
    let mut t_release: Option<Instant> = None;
    let mut chunks_before_release = 0usize;
    for msg in rx.iter() {
        match msg {
            Msg::Audio(mut samples) => {
                if let (Ok(e), None) = (&mut loaded, &failure) {
                    match e.stream_feed(&samples, hw) {
                        Ok(()) if !released.load(Ordering::Relaxed) => chunks_before_release = e.stream_chunks(),
                        Ok(()) => {}
                        Err(err) => failure = Some(err),
                    }
                }
                samples.zeroize();
            }
            Msg::Finish(t) => {
                t_release = Some(t);
                break;
            }
        }
    }
    let mut engine = match loaded {
        Ok(e) => e,
        Err(err) => return LiveDone::failed(None, err),
    };
    let Some(t_release) = t_release else {
        engine.stream_reset();
        return LiveDone::failed(Some(engine), anyhow!("recording cancelled"));
    };
    let result = match failure {
        Some(err) => {
            engine.stream_reset();
            Err(err)
        }
        None => engine.stream_finish(hw),
    };
    LiveDone {
        engine: Some(engine),
        result,
        model_wait_ms: t_loaded.saturating_duration_since(t_release).as_secs_f64() * 1000.0,
        post_release_ms: t_release.elapsed().as_secs_f64() * 1000.0,
        chunks_before_release,
    }
}
