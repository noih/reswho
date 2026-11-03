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
    settings: Option<Arc<Mutex<Arc<crate::settings::Resolved>>>>,
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
        let mut worker = Self::with_settings_collector(
            Arc::clone(&options.settings),
            move |cancel, command_pid, settings| {
                options.settings = settings;
                options.command_pid = command_pid;
                snapshot::capture_reported(&options, cancel, Arc::clone(&current))
                    .map_err(|_| "Collection failed; see current extension status".into())
            },
        );
        worker.report = report;
        worker
    }

    fn with_settings_collector(
        initial: Arc<crate::settings::Resolved>,
        mut collect: impl FnMut(
            &Cancellation,
            Option<u32>,
            Arc<crate::settings::Resolved>,
        ) -> Result<Snapshot, String>
        + Send
        + 'static,
    ) -> Self {
        let settings = Arc::new(Mutex::new(initial));
        let latest = Arc::clone(&settings);
        let mut worker = Self::with_collector(move |cancel, command_pid| {
            let settings = Arc::clone(&latest.lock().expect("settings lock"));
            collect(cancel, command_pid, settings)
        });
        worker.settings = Some(settings);
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
            settings: None,
            cancel,
            handle: Some(handle),
        }
    }

    pub fn set_settings(&self, settings: Arc<crate::settings::Resolved>) {
        if let Some(latest) = &self.settings {
            *latest.lock().expect("settings lock") = settings;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{Config, Registry};
    use std::{collections::BTreeMap, time::Duration};

    #[test]
    fn in_flight_keeps_settings_and_next_request_uses_saved_settings() {
        let resolved = |interval| {
            Arc::new(
                Registry::core()
                    .resolve(
                        &Config {
                            source_priority: vec![format!("source_{interval}")],
                            ..Default::default()
                        },
                        &BTreeMap::from([(
                            "sampling.interval_ms".into(),
                            serde_json::json!(interval),
                        )]),
                    )
                    .unwrap(),
            )
        };
        let (entered, started) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let worker = Worker::with_settings_collector(resolved(250), move |_, _, settings| {
            entered
                .send((
                    settings.interval("sampling.interval_ms"),
                    settings.source_priority.clone(),
                ))
                .unwrap();
            wait.recv_timeout(Duration::from_secs(2)).unwrap();
            entered
                .send((
                    settings.interval("sampling.interval_ms"),
                    settings.source_priority.clone(),
                ))
                .unwrap();
            Err("fixture".into())
        });
        assert!(worker.request(None));
        assert_eq!(
            started.recv_timeout(Duration::from_secs(2)).unwrap(),
            (Duration::from_millis(250), vec!["source_250".into()])
        );
        worker.set_settings(resolved(500));
        release.send(()).unwrap();
        assert_eq!(
            started.recv_timeout(Duration::from_secs(2)).unwrap(),
            (Duration::from_millis(250), vec!["source_250".into()])
        );
        assert!(
            worker
                .results
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .is_err()
        );
        assert!(worker.request(None));
        assert_eq!(
            started.recv_timeout(Duration::from_secs(2)).unwrap(),
            (Duration::from_millis(500), vec!["source_500".into()])
        );
        release.send(()).unwrap();
        assert_eq!(
            started.recv_timeout(Duration::from_secs(2)).unwrap(),
            (Duration::from_millis(500), vec!["source_500".into()])
        );
        assert!(
            worker
                .results
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .is_err()
        );
        drop(worker);
    }
}
