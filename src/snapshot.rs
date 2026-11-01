use crate::{collect, containers, model::Snapshot, tree};
use anyhow::{Context, Result, bail};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);

impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    pub fn check(&self) -> Result<()> {
        if self.0.load(Ordering::Relaxed) {
            bail!("collection canceled");
        }
        Ok(())
    }
    pub fn wait(&self, duration: Duration) -> Result<()> {
        let start = Instant::now();
        loop {
            self.check()?;
            let remaining = duration.saturating_sub(start.elapsed());
            if remaining.is_zero() {
                return Ok(());
            }
            std::thread::sleep(remaining.min(Duration::from_millis(20)));
        }
    }
}

#[derive(Clone)]
pub struct Options {
    pub sample_interval: Duration,
    pub containers: bool,
}

pub fn capture(options: &Options, cancel: &Cancellation) -> Result<Snapshot> {
    let start = Instant::now();
    cancel.check()?;
    let mut collection = collect::collect_with_cancel(options.sample_interval, cancel)?;
    cancel.check()?;
    let mut root = tree::build(collection.processes)?;
    if options.containers {
        cancel.check()?;
        match containers::collect_with_cancel(cancel, options.sample_interval) {
            Ok(containers) => root.children.push(containers),
            Err(error) => collection.warnings.push(crate::model::Diagnostic::new(
                "collection_failed",
                "containers",
                format!("Containers: {error}"),
            )),
        }
    }
    cancel.check()?;
    tree::validate_references(&root)?;
    Ok(Snapshot {
        schema: crate::model::JSON_SCHEMA_VERSION,
        captured_unix_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock before Unix epoch")?
            .as_millis() as u64,
        sample_ms: collection.sample_ms,
        collection_ms: start.elapsed().as_millis() as u64,
        system: collection.system,
        scope: "group:all".into(),
        warnings: collection.warnings,
        root,
    })
}
