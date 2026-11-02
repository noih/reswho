use super::*;
use std::{cell::Cell, rc::Rc};

fn sample(id: &str, cpu: Option<u64>, ns: u64) -> Sample {
    Sample {
        counters: Counters {
            id: id.into(),
            cpu_usage_usec: cpu,
            memory_usage_bytes: Some(100),
        },
        received_ns: ns,
    }
}
#[test]
fn counters_use_actual_interval_and_preserve_unknown_cpu() {
    let first = sample("a", Some(1_000_000), 1_000_000_000);
    let second = sample("a", Some(1_500_000), 1_250_000_000);
    assert_eq!(metrics(Some(&first), &second).cpu_percent, Some(200.0));
    assert_eq!(metrics(Some(&second), &first).cpu_percent, None);
    assert_eq!(
        metrics(Some(&first), &sample("a", Some(0), 2_000_000_000)).cpu_percent,
        None
    );
    assert_eq!(metrics(None, &second).memory_bytes, Some(100));
    assert_eq!(metrics(None, &second).cpu_percent, None);
    assert_eq!(
        metrics(Some(&first), &sample("a", None, 2_000_000_000)).cpu_percent,
        None
    );
    assert!(serde_json::from_str::<Counters>(r#"{"id":"a","cpuUsageUsec":-1}"#).is_err());
}

struct Fake {
    id: String,
    active: Rc<Cell<usize>>,
    pending: bool,
}
impl Drop for Fake {
    fn drop(&mut self) {
        self.active.set(self.active.get() - 1);
    }
}
impl Pending for Fake {
    fn poll(&mut self) -> Result<Option<Sample>> {
        match self.id.as_str() {
            "bad" => bail!("failure"),
            "wrong" => Ok(Some(sample("different", Some(1), 1))),
            _ if self.pending => Ok(None),
            _ => Ok(Some(sample(&self.id, Some(1), 1))),
        }
    }
}
#[test]
fn bounded_concurrency_partial_failure_deadline_and_cancellation_drop_requests() {
    let active = Rc::new(Cell::new(0));
    let peak = Cell::new(0);
    let make = |id: &str, pending| {
        active.set(active.get() + 1);
        peak.set(peak.get().max(active.get()));
        Ok(Fake {
            id: id.into(),
            active: active.clone(),
            pending,
        })
    };
    let ids = vec![
        "a".into(),
        "bad".into(),
        "wrong".into(),
        "b".into(),
        "c".into(),
        "d".into(),
    ];
    let result = pass(
        &ids,
        &Cancellation::default(),
        Instant::now() + Duration::from_secs(1),
        |id| make(id, false),
    )
    .unwrap();
    assert_eq!(result.len(), 4);
    assert_eq!(peak.get(), 4);
    assert_eq!(active.get(), 0);
    let start = Instant::now();
    assert!(
        pass(
            &ids,
            &Cancellation::default(),
            start + Duration::from_millis(15),
            |id| make(id, true)
        )
        .unwrap()
        .is_empty()
    );
    assert!(start.elapsed() < Duration::from_secs(1));
    assert_eq!(active.get(), 0);
    let cancel = Cancellation::default();
    let result = pass(
        &ids,
        &cancel,
        Instant::now() + Duration::from_secs(1),
        |id| {
            cancel.cancel();
            make(id, true)
        },
    );
    assert!(result.is_err());
    assert_eq!(active.get(), 0);
}
