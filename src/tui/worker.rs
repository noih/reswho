use crate::{
    model::{Node, Snapshot},
    snapshot::{self, Cancellation, DetailSnapshot, Options},
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

pub(super) enum Collected {
    Overview(Snapshot),
    Detail(DetailSnapshot),
}
pub(super) struct Reply {
    pub cancelled: bool,
    pub detail_id: Option<u64>,
    pub result: Result<Collected, String>,
}
struct Request {
    target: Option<(u64, Node)>,
    cancel: Cancellation,
}

pub(super) struct Worker {
    requests: SyncSender<Request>,
    results: Receiver<Reply>,
    pub report: Arc<Mutex<Vec<crate::extensions::report::Record>>>,
    inputs: Option<Arc<Mutex<Inputs>>>,
    cancel: Cancellation,
    active: Mutex<Option<Cancellation>>,
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
            move |cancel, target, inputs| {
                options.settings = inputs.settings;
                if let Some(target) = target {
                    snapshot::capture_detail(&options, cancel, &target, Arc::clone(&current))
                        .map(Collected::Detail)
                } else {
                    options.command_pids.clear();
                    options.detail_refs.clear();
                    snapshot::capture_reported(&options, cancel, Arc::clone(&current))
                        .map(Collected::Overview)
                }
                .map_err(|_| "Collection failed; see current extension status".into())
            },
        );
        worker.report = report;
        worker
    }

    fn with_inputs_collector(
        initial: Inputs,
        mut collect: impl FnMut(&Cancellation, Option<Node>, Inputs) -> Result<Collected, String>
        + Send
        + 'static,
    ) -> Self {
        let inputs = Arc::new(Mutex::new(initial));
        let latest = Arc::clone(&inputs);
        let mut worker = Self::with_collector(move |cancel, target| {
            let inputs = latest.lock().expect("inputs lock").clone();
            collect(cancel, target, inputs)
        });
        worker.inputs = Some(inputs);
        worker
    }

    pub(super) fn with_collector(
        mut collect: impl FnMut(&Cancellation, Option<Node>) -> Result<Collected, String>
        + Send
        + 'static,
    ) -> Self {
        let (requests, incoming) = mpsc::sync_channel::<Request>(1);
        let (outgoing, results) = mpsc::sync_channel(1);
        let cancel = Cancellation::default();
        let stopping = cancel.clone();
        let handle = thread::spawn(move || {
            while let Ok(request) = incoming.recv() {
                if stopping.check().is_err() {
                    break;
                }
                let detail_id = request.target.as_ref().map(|(id, _)| *id);
                let result = collect(&request.cancel, request.target.map(|(_, node)| node));
                if stopping.check().is_err() {
                    break;
                }
                if outgoing
                    .try_send(Reply {
                        cancelled: request.cancel.check().is_err(),
                        detail_id,
                        result,
                    })
                    .is_err()
                {
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
            active: Mutex::default(),
            handle: Some(handle),
        }
    }

    pub fn set_inputs(&self, inputs: Inputs) {
        if let Some(latest) = &self.inputs {
            *latest.lock().expect("inputs lock") = inputs;
        }
    }

    pub fn request(&self, target: Option<(u64, Node)>) -> bool {
        let mut active = self.active.lock().expect("active request lock");
        if active.is_some() {
            return false;
        }
        let cancel = Cancellation::default();
        if self
            .requests
            .try_send(Request {
                target,
                cancel: cancel.clone(),
            })
            .is_err()
        {
            return false;
        }
        *active = Some(cancel);
        true
    }

    pub fn try_recv(&self) -> Result<Reply, mpsc::TryRecvError> {
        let reply = self.results.try_recv()?;
        self.active.lock().expect("active request lock").take();
        Ok(reply)
    }

    #[cfg(test)]
    pub fn recv_timeout(
        &self,
        timeout: std::time::Duration,
    ) -> Result<Reply, mpsc::RecvTimeoutError> {
        let reply = self.results.recv_timeout(timeout)?;
        self.active.lock().expect("active request lock").take();
        Ok(reply)
    }

    pub fn cancel_active(&self) {
        if let Some(cancel) = self.active.lock().expect("active request lock").as_ref() {
            cancel.cancel();
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.cancel_active();
        let _ = self.requests.try_send(Request {
            target: None,
            cancel: Cancellation::default(),
        });
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
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .result
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
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .result
                .is_err()
        );
        drop(worker);
    }
}
