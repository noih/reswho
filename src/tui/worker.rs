use crate::{
    model::Snapshot,
    snapshot::{self, Cancellation, Options},
};
use std::{
    sync::mpsc::{self, Receiver, SyncSender},
    thread::{self, JoinHandle},
};

pub(super) struct Worker {
    requests: SyncSender<()>,
    pub results: Receiver<Result<Snapshot, String>>,
    cancel: Cancellation,
    handle: Option<JoinHandle<()>>,
}

impl Worker {
    pub fn start(options: Options) -> Self {
        Self::with_collector(move |cancel| {
            snapshot::capture(&options, cancel).map_err(|error| format!("{error:#}"))
        })
    }

    pub(super) fn with_collector(
        mut collect: impl FnMut(&Cancellation) -> Result<Snapshot, String> + Send + 'static,
    ) -> Self {
        let (requests, incoming) = mpsc::sync_channel(1);
        let (outgoing, results) = mpsc::sync_channel(1);
        let cancel = Cancellation::default();
        let stopping = cancel.clone();
        let handle = thread::spawn(move || {
            while incoming.recv().is_ok() && stopping.check().is_ok() {
                let result = collect(&stopping);
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
            cancel,
            handle: Some(handle),
        }
    }

    pub fn request(&self) -> bool {
        self.requests.try_send(()).is_ok()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel.cancel();
        let _ = self.requests.try_send(());
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
