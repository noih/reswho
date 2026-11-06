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

#[derive(Clone)]
pub(super) struct Inputs {
    pub settings: Arc<crate::settings::Resolved>,
}

pub(super) struct Worker {
    requests: SyncSender<Option<String>>,
    pub results: Receiver<Result<Snapshot, String>>,
    pub report: Arc<Mutex<Vec<crate::extensions::report::Record>>>,
    inputs: Option<Arc<Mutex<Inputs>>>,
    cancel: Cancellation,
    handle: Option<JoinHandle<()>>,
}

impl Worker {
    pub fn start(mut options: Options) -> Self {
        let report = Arc::new(Mutex::new(crate::extensions::inventory::initial_report(
            &options.settings,
            options.containers,
        )));
        let current = Arc::clone(&report);
        let mut worker = Self::with_inputs_collector(
            Inputs {
                settings: Arc::clone(&options.settings),
            },
            move |cancel, detail_ref, inputs| {
                options.settings = inputs.settings;
                options.command_pids = detail_ref
                    .as_deref()
                    .and_then(crate::model::selector_pid)
                    .into_iter()
                    .collect();
                options.detail_refs = detail_ref.into_iter().collect();
                snapshot::capture_reported(&options, cancel, Arc::clone(&current))
                    .map_err(|_| "Collection failed; see current extension status".into())
            },
        );
        worker.report = report;
        worker
    }

    fn with_inputs_collector(
        initial: Inputs,
        mut collect: impl FnMut(&Cancellation, Option<String>, Inputs) -> Result<Snapshot, String>
        + Send
        + 'static,
    ) -> Self {
        let inputs = Arc::new(Mutex::new(initial));
        let latest = Arc::clone(&inputs);
        let mut worker = Self::with_collector(move |cancel, command_pid| {
            let inputs = latest.lock().expect("inputs lock").clone();
            collect(cancel, command_pid, inputs)
        });
        worker.inputs = Some(inputs);
        worker
    }

    pub(super) fn with_collector(
        mut collect: impl FnMut(&Cancellation, Option<String>) -> Result<Snapshot, String>
        + Send
        + 'static,
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
            inputs: None,
            cancel,
            handle: Some(handle),
        }
    }

    pub fn set_inputs(&self, inputs: Inputs) {
        if let Some(latest) = &self.inputs {
            *latest.lock().expect("inputs lock") = inputs;
        }
    }

    pub fn request(&self, detail_ref: Option<String>) -> bool {
        self.requests.try_send(detail_ref).is_ok()
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
    fn in_flight_keeps_inputs_and_next_request_uses_saved_inputs() {
        let resolved = |interval| {
            Arc::new(
                Registry::core()
                    .resolve(
                        &Config {
                            disabled_builtins: if interval == 500 {
                                vec!["project".into()]
                            } else {
                                vec![]
                            },
                            ownership_priority: vec![format!("dimension_{interval}")],
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
        let worker = Worker::with_inputs_collector(
            Inputs {
                settings: resolved(250),
            },
            move |_, _, inputs| {
                let settings = inputs.settings;
                entered
                    .send((
                        settings.interval("sampling.interval_ms"),
                        settings.source_priority.clone(),
                        settings.ownership_priority.clone(),
                        settings.is_disabled("project"),
                    ))
                    .unwrap();
                wait.recv_timeout(Duration::from_secs(2)).unwrap();
                entered
                    .send((
                        settings.interval("sampling.interval_ms"),
                        settings.source_priority.clone(),
                        settings.ownership_priority.clone(),
                        settings.is_disabled("project"),
                    ))
                    .unwrap();
                Err("fixture".into())
            },
        );
        assert!(worker.request(None));
        assert_eq!(
            started.recv_timeout(Duration::from_secs(2)).unwrap(),
            (
                Duration::from_millis(250),
                vec!["source_250".into()],
                vec!["dimension_250".into()],
                false,
            )
        );
        worker.set_inputs(Inputs {
            settings: resolved(500),
        });
        release.send(()).unwrap();
        assert_eq!(
            started.recv_timeout(Duration::from_secs(2)).unwrap(),
            (
                Duration::from_millis(250),
                vec!["source_250".into()],
                vec!["dimension_250".into()],
                false,
            )
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
            (
                Duration::from_millis(500),
                vec!["source_500".into()],
                vec!["dimension_500".into()],
                true,
            )
        );
        release.send(()).unwrap();
        assert_eq!(
            started.recv_timeout(Duration::from_secs(2)).unwrap(),
            (
                Duration::from_millis(500),
                vec!["source_500".into()],
                vec!["dimension_500".into()],
                true,
            )
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
