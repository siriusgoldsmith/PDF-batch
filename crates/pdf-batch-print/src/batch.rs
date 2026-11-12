//! Batch worker: port of the `work()` loop in `app.py::start`. Runs on its own
//! thread and reports to the UI through a channel; the UI never blocks on it.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::Duration;

use pdf_batch_core::QueueStore;

use crate::{Callbacks, PrintError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchEvent {
    Status(String),
    /// Queue rows changed; refresh the list.
    Refresh,
    /// `(completed, target)` documents finished according to Windows.
    Batch(usize, usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchOutcome {
    Finished { completed: usize, target: usize },
    Paused { completed: usize, target: usize },
    /// Stopped safely; the item (if any) is `uncertain` and needs operator review.
    Error(String),
}

/// Signature of the document printer, so tests can inject a fake transport.
pub type PrintFn<'a> = dyn FnMut(&Path, &str, u32, Callbacks, &AtomicBool) -> Result<(), PrintError> + 'a;

/// Print up to `target` pending documents in order. `pause` takes effect BETWEEN
/// documents: a document is never cut by a user pause.
pub fn run_batch(
    store: &QueueStore,
    printer: &str,
    target: usize,
    pause: &AtomicBool,
    events: &Sender<BatchEvent>,
    print: &mut PrintFn,
) -> BatchOutcome {
    let mut completed = 0usize;
    let send = |e: BatchEvent| {
        let _ = events.send(e);
    };
    while !pause.load(Ordering::SeqCst) && completed < target {
        let item = match store.pending() {
            Ok(Some(i)) => i,
            Ok(None) => break,
            Err(e) => return BatchOutcome::Error(e.to_string()),
        };
        let id = item.id;
        let start = item.start_page.max(1) as u32;
        if let Err(e) = store.claim(id, printer) {
            send(BatchEvent::Refresh);
            return BatchOutcome::Error(e.to_string());
        }
        send(BatchEvent::Status(format!("Enviando #{}: {}", item.position, item.name)));
        let document_stop = AtomicBool::new(false);
        let mut on_job = |job: u32, total: u32| {
            store
                .progress(id, start as i64 - 1, Some(total as i64), Some(&job.to_string()))
                .map_err(|e| e.to_string())
        };
        let mut on_progress = |page: u32, total: u32| {
            let r = store.progress(id, page as i64, Some(total as i64), None).map_err(|e| e.to_string());
            send(BatchEvent::Refresh);
            r
        };
        let result = print(
            Path::new(&item.snapshot),
            printer,
            start,
            Callbacks { on_job: &mut on_job, on_progress: &mut on_progress },
            &document_stop,
        );
        let saved = match &result {
            Ok(()) => store.mark_sent(id),
            Err(e) => store.fail(id, &e.to_string()),
        };
        send(BatchEvent::Refresh);
        match (result, saved) {
            (Ok(()), Ok(())) => {
                completed += 1;
                send(BatchEvent::Batch(completed, target));
            }
            (Err(e), Ok(())) => return BatchOutcome::Error(e.to_string()),
            (Ok(()), Err(s)) => return BatchOutcome::Error(format!("No se pudo guardar el estado: {s}")),
            (Err(e), Err(s)) => return BatchOutcome::Error(format!("{e}; además no se pudo guardar el estado: {s}")),
        }
    }
    if pause.load(Ordering::SeqCst) {
        BatchOutcome::Paused { completed, target }
    } else {
        BatchOutcome::Finished { completed, target }
    }
}

/// Real transport with the v1.x 600 s queue timeout.
#[cfg(windows)]
pub fn windows_printer() -> impl FnMut(&Path, &str, u32, Callbacks, &AtomicBool) -> Result<(), PrintError> {
    |p, printer, start, cb, stop| crate::print_document(p, printer, start, cb, stop, Duration::from_secs(600))
}

