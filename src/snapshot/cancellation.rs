use anyhow::{Result, bail};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
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
