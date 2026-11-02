use crate::{
    model::Snapshot,
    snapshot::{self, Cancellation, Options},
};
use std::{
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
};

pub(super) struct Worker {
    requests: SyncSender<Option<u32>>,
    pub results: Receiver<Result<Snapshot, String>>,
    pub report: Arc<Mutex<Vec<crate::extensions::report::Record>>>,
    cancel: Cancellation,
    handle: Option<JoinHandle<()>>,
}

impl Worker {
    pub fn start(mut options: Options) -> Self {
        let report = Arc::new(Mutex::new(
            options
                .extensions
                .initial_report(&options.settings, options.containers),
        ));
        let current = Arc::clone(&report);
        let mut worker = Self::with_collector(move |cancel, command_pid| {
            options.command_pid = command_pid;
            snapshot::capture_reported(&options, cancel, Arc::clone(&current))
                .map_err(|_| "Collection failed; see current extension status".into())
        });
        worker.report = report;
        worker
    }

    pub(super) fn with_collector(
        mut collect: impl FnMut(&Cancellation, Option<u32>) -> Result<Snapshot, String> + Send + 'static,
    ) -> Self {
        let (requests, incoming) = mpsc::sync_channel(1);
        let (outgoing, results) = mpsc::sync_channel(1);
        let cancel = Cancellation::default();
        let stopping = cancel.clone();
        let handle = thread::spawn(move || {
            while let Ok(command_pid) = incoming.recv() {
                if stopping.check().is_err() {
                    break;
                }
                let result = collect(&stopping, command_pid);
                if stopping.check().is_err() {
                    break;
                }
                if outgoing.try_send(result).is_err() {
                    break;
                }
            }
        });
        Self {
            requests,
            results,
            report: Arc::default(),
            cancel,
            handle: Some(handle),
        }
    }

    pub fn request(&self, command_pid: Option<u32>) -> bool {
        self.requests.try_send(command_pid).is_ok()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel.cancel();
        let _ = self.requests.try_send(None);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
