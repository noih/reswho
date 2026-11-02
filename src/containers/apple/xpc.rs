use super::MEMORY_BASIS;
use crate::{
    containers::stats::{Batch, Metrics},
    model::Diagnostic,
    snapshot::Cancellation,
};
use anyhow::{Result, bail};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    ffi::{CString, c_char, c_void},
    ptr::NonNull,
    time::{Duration, Instant},
};

unsafe extern "C" {
    fn reswho_apple_stats_start(id: *const c_char) -> *mut c_void;
    fn reswho_apple_stats_poll(
        handle: *mut c_void,
        data: *mut *const c_void,
        size: *mut usize,
        received_ns: *mut u64,
    ) -> i32;
    fn reswho_apple_stats_drop(handle: *mut c_void);
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Counters {
    id: String,
    cpu_usage_usec: Option<u64>,
    memory_usage_bytes: Option<u64>,
}

struct Sample {
    counters: Counters,
    received_ns: u64,
}

trait Pending {
    fn poll(&mut self) -> Result<Option<Sample>>;
}

struct Request(NonNull<c_void>);

impl Request {
    fn start(id: &str) -> Result<Self> {
        let id = CString::new(id)?;
        let handle = unsafe { reswho_apple_stats_start(id.as_ptr()) };
        NonNull::new(handle)
            .map(Self)
            .ok_or_else(|| anyhow::anyhow!("cannot allocate XPC request"))
    }
}

impl Drop for Request {
    fn drop(&mut self) {
        unsafe { reswho_apple_stats_drop(self.0.as_ptr()) };
    }
}

impl Pending for Request {
    fn poll(&mut self) -> Result<Option<Sample>> {
        let mut data = std::ptr::null();
        let mut size = 0;
        let mut received_ns = 0;
        match unsafe {
            reswho_apple_stats_poll(self.0.as_ptr(), &mut data, &mut size, &mut received_ns)
        } {
            0 => Ok(None),
            1 if !data.is_null() && size > 0 && size <= 65536 && received_ns > 0 => {
                // The native request owns this immutable buffer until Drop, including late replies.
                let bytes = unsafe { std::slice::from_raw_parts(data.cast::<u8>(), size) };
                let counters = serde_json::from_slice(bytes)
                    .map_err(|_| anyhow::anyhow!("invalid XPC stats JSON"))?;
                Ok(Some(Sample {
                    counters,
                    received_ns,
                }))
            }
            _ => bail!("XPC stats unavailable or invalid"),
        }
    }
}

fn pass<P: Pending>(
    ids: &[String],
    cancel: &Cancellation,
    deadline: Instant,
    mut start: impl FnMut(&str) -> Result<P>,
) -> Result<BTreeMap<String, Sample>> {
    let mut waiting = ids.iter();
    let mut active = Vec::new();
    let mut samples = BTreeMap::new();
    loop {
        cancel.check()?;
        if Instant::now() >= deadline {
            break;
        }
        while active.len() < 4 {
            let Some(id) = waiting.next() else {
                break;
            };
            if let Ok(request) = start(id) {
                active.push((id, request, Instant::now()));
            }
        }
        if active.is_empty() {
            break;
        }
        let mut i = 0;
        while i < active.len() {
            let (id, request, started) = &mut active[i];
            match request.poll() {
                Ok(Some(sample)) => {
                    if sample.counters.id == **id {
                        samples.insert((*id).clone(), sample);
                    }
                }
                Ok(None) if started.elapsed() < Duration::from_secs(2) => {
                    i += 1;
                    continue;
                }
                Ok(None) | Err(_) => {}
            }
            active.swap_remove(i);
        }
        if !active.is_empty() {
            cancel.wait(Duration::from_millis(5))?;
        }
    }
    Ok(samples)
}

fn metrics(first: Option<&Sample>, second: &Sample) -> Metrics {
    let cpu_percent = first
        .and_then(|first| {
            let elapsed = second.received_ns.checked_sub(first.received_ns)?;
            let delta = second
                .counters
                .cpu_usage_usec?
                .checked_sub(first.counters.cpu_usage_usec?)?;
            (elapsed > 0).then(|| delta as f64 * 100_000.0 / elapsed as f64)
        })
        .filter(|value| value.is_finite());
    Metrics {
        cpu_percent,
        memory_bytes: second.counters.memory_usage_bytes,
    }
}

pub(super) fn collect(
    ids: &[String],
    cancel: &Cancellation,
    deadline: Instant,
    interval: Duration,
) -> Result<Batch> {
    let first = pass(ids, cancel, deadline, Request::start)?;
    cancel.wait(interval.min(deadline.saturating_duration_since(Instant::now())))?;
    let second = pass(ids, cancel, deadline, Request::start)?;
    let metrics = second
        .iter()
        .map(|(id, sample)| (id.clone(), metrics(first.get(id), sample)))
        .collect();
    Ok(Batch {
        metrics,
        memory_basis: MEMORY_BASIS,
        unavailable: if Instant::now() >= deadline {
            vec![Diagnostic::new(
                "unavailable",
                "apple.stats",
                "stats deadline exceeded",
            )]
        } else {
            Vec::new()
        },
    })
}

#[cfg(test)]
mod tests;
