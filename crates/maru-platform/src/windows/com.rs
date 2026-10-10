//! A thread that owns COM/WinRT objects.
//!
//! UI Automation and WinRT objects are created and used on one MTA thread; the
//! rest of Maru talks to it through a channel. A hung target application can
//! block a COM call, so callers always wait with a timeout.

use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::Duration;

use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};

use crate::{PlatformError, Result};

type Job<S> = Box<dyn FnOnce(&S) + Send>;

pub struct ComWorker<S: 'static> {
    tx: Sender<Job<S>>,
}

impl<S: 'static> ComWorker<S> {
    /// `init` runs on the new thread and builds the state that lives there.
    pub fn start(name: &str, init: impl FnOnce() -> windows::core::Result<S> + Send + 'static) -> Result<Self> {
        let (tx, rx) = mpsc::channel::<Job<S>>();
        let (ready_tx, ready_rx) = mpsc::channel::<std::result::Result<(), String>>();
        thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                // SAFETY: standard COM initialisation for this thread; balanced below.
                let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
                if com.is_err() {
                    let _ = ready_tx.send(Err(format!("CoInitializeEx failed: {com:?}")));
                    return;
                }
                match init() {
                    Ok(state) => {
                        let _ = ready_tx.send(Ok(()));
                        while let Ok(job) = rx.recv() {
                            job(&state);
                        }
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e.to_string()));
                    }
                }
                // SAFETY: matches the successful CoInitializeEx above.
                unsafe { CoUninitialize() };
            })
            .map_err(|e| PlatformError::Failed(e.to_string()))?;
        match ready_rx.recv_timeout(Duration::from_secs(20)) {
            Ok(Ok(())) => Ok(Self { tx }),
            Ok(Err(e)) => Err(PlatformError::Failed(e)),
            Err(mpsc::RecvTimeoutError::Timeout) => Err(PlatformError::Failed(format!("{name} did not start within 20 seconds"))),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(PlatformError::Failed("COM thread exited during start-up".into())),
        }
    }

    pub fn call<T: Send + 'static>(&self, timeout: Duration, f: impl FnOnce(&S) -> T + Send + 'static) -> Result<T> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx
            .send(Box::new(move |s| {
                let _ = reply_tx.send(f(s));
            }))
            .map_err(|_| PlatformError::Failed("COM thread is gone".into()))?;
        reply_rx.recv_timeout(timeout).map_err(|_| PlatformError::Failed(format!("no answer from the target application within {timeout:?}")))
    }
}
