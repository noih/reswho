use crate::snapshot::Cancellation;
use anyhow::{Context, Result, bail};
use curl::{
    easy::{Easy2, Handler, WriteError},
    multi::Multi,
};
use std::{
    path::Path,
    time::{Duration, Instant},
};

const MAX_BODY_BYTES: usize = 1024 * 1024;
const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_CONCURRENT: usize = 4;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
const POLL_INTERVAL: Duration = Duration::from_millis(20);

#[derive(Default)]
struct Body {
    bytes: Vec<u8>,
    header_bytes: usize,
    exceeded: bool,
}

#[cfg(test)]
pub(super) mod tests;

impl Handler for Body {
    fn write(&mut self, data: &[u8]) -> Result<usize, WriteError> {
        if self.bytes.len() + data.len() > MAX_BODY_BYTES {
            self.exceeded = true;
            return Ok(0);
        }
        self.bytes.extend_from_slice(data);
        Ok(data.len())
    }

    fn header(&mut self, data: &[u8]) -> bool {
        self.header_bytes += data.len();
        self.exceeded |= self.header_bytes > MAX_HEADER_BYTES;
        !self.exceeded
    }
}

pub(super) struct Response {
    pub status: u32,
    pub body: Vec<u8>,
}

impl Response {
    pub fn json<T: serde::de::DeserializeOwned>(self) -> Result<T> {
        if self.status != 200 {
            bail!("Docker API HTTP {}; response withheld", self.status);
        }
        serde_json::from_slice(&self.body).map_err(|_| anyhow::anyhow!("invalid Docker API JSON"))
    }
}

pub(super) struct Client<'a> {
    pub socket: &'a Path,
    pub cancel: &'a Cancellation,
    pub deadline: Instant,
}

impl Client<'_> {
    pub fn check(&self) -> Result<()> {
        self.cancel.check()?;
        if Instant::now() >= self.deadline {
            bail!("Docker collection deadline exceeded");
        }
        Ok(())
    }

    pub fn wait(&self, interval: Duration) -> Result<()> {
        self.check()?;
        self.cancel
            .wait(interval.min(self.deadline.saturating_duration_since(Instant::now())))?;
        self.check()
    }

    pub fn get(&self, path: &str) -> Result<Response> {
        self.batch(&[path.to_owned()])
            .next()
            .context("Docker API response missing")?
    }

    pub fn batch<'a>(&'a self, paths: &'a [String]) -> impl Iterator<Item = Result<Response>> + 'a {
        paths
            .chunks(MAX_CONCURRENT)
            .flat_map(|chunk| match self.request_chunk(chunk) {
                Ok(results) => results,
                Err(error) => {
                    let message = error.to_string();
                    (0..chunk.len())
                        .map(|_| Err(anyhow::anyhow!(message.clone())))
                        .collect()
                }
            })
    }

    fn request_chunk(&self, paths: &[String]) -> Result<Vec<Result<Response>>> {
        self.check()?;
        let multi = Multi::new();
        let mut handles = Vec::with_capacity(paths.len());
        let mut collection_limited = Vec::with_capacity(paths.len());
        for path in paths {
            let mut request = Easy2::new(Body::default());
            request.url(&format!("http://localhost{path}"))?;
            request.unix_socket_path(Some(self.socket))?;
            request.proxy("")?;
            request.follow_location(false)?;
            let remaining = self.deadline.saturating_duration_since(Instant::now());
            request.timeout(REQUEST_TIMEOUT.min(remaining).max(Duration::from_millis(1)))?;
            collection_limited.push(remaining <= REQUEST_TIMEOUT);
            handles.push(multi.add2(request)?);
        }
        let mut completed = vec![None; handles.len()];
        loop {
            self.check()?;
            let running = multi.perform()?;
            multi.messages(|message| {
                for (index, handle) in handles.iter().enumerate() {
                    if let Some(result) = message.result_for2(handle) {
                        completed[index] = Some(result.map_err(|error| {
                            if error.is_operation_timedout() {
                                if collection_limited[index] {
                                    "Docker collection deadline exceeded".into()
                                } else {
                                    "Docker API request timed out".into()
                                }
                            } else {
                                format!(
                                    "Docker API transport failed (curl code {}); details withheld",
                                    error.code()
                                )
                            }
                        }));
                    }
                }
            });
            if running == 0 {
                break;
            }
            multi.wait(&mut [], POLL_INTERVAL)?;
        }
        Ok(handles
            .into_iter()
            .zip(completed)
            .map(|(handle, result)| {
                let mut request = multi.remove2(handle)?;
                if request.get_ref().exceeded {
                    bail!("Docker API response exceeds size limit");
                }
                match result {
                    Some(Ok(())) => Ok(Response {
                        status: request.response_code()?,
                        body: std::mem::take(&mut request.get_mut().bytes),
                    }),
                    Some(Err(message)) => {
                        bail!("{message}")
                    }
                    None => bail!("Docker API transfer incomplete"),
                }
            })
            .collect())
    }
}
