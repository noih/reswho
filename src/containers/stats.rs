use super::http::Client;
use crate::model::Diagnostic;
use anyhow::{Result, bail};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

pub(super) const CLI_MEMORY_BASIS: &str = "docker_cli_linux_cache_adjusted_rounded";
pub(super) const API_MEMORY_BASIS: &str = "docker_api_linux_cache_adjusted";

#[derive(Debug, Default)]
pub(super) struct Metrics {
    pub cpu_percent: Option<f64>,
    pub memory_bytes: Option<u64>,
}

pub(super) struct Batch {
    pub metrics: BTreeMap<String, Metrics>,
    pub memory_basis: &'static str,
    pub unavailable: Vec<Diagnostic>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Version {
    api_version: String,
    min_api_version: Option<String>,
    os: String,
}

fn version(value: &str) -> Option<(u16, u16)> {
    let (major, minor) = value.split_once('.')?;
    Some((major.parse().ok()?, minor.parse().ok()?))
}

fn negotiated(server: &Version, requested: Option<&str>) -> Option<String> {
    let maximum = version(&server.api_version)?.min((1, 56));
    let minimum = server
        .min_api_version
        .as_deref()
        .map(version)
        .unwrap_or(Some((1, 24)))?
        .max((1, 41));
    let chosen = match requested.filter(|value| !value.is_empty()) {
        Some(value) => version(value)?,
        None => maximum,
    };
    (server.os == "linux" && chosen >= minimum && chosen <= maximum)
        .then(|| format!("v{}.{}", chosen.0, chosen.1))
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct CpuUsage {
    total_usage: Option<u64>,
    percpu_usage: Option<Vec<u64>>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Cpu {
    cpu_usage: CpuUsage,
    system_cpu_usage: Option<u64>,
    online_cpus: Option<u64>,
}

impl Cpu {
    fn cores(&self) -> Option<u64> {
        self.online_cpus.filter(|value| *value > 0).or_else(|| {
            self.cpu_usage
                .percpu_usage
                .as_ref()
                .map(|values| values.len() as u64)
                .filter(|value| *value > 0)
        })
    }

    fn percent_since(&self, previous: &Self) -> Option<f64> {
        let cores = self.cores()?;
        if previous.cores()? != cores {
            return None;
        }
        let cpu = self
            .cpu_usage
            .total_usage?
            .checked_sub(previous.cpu_usage.total_usage?)?;
        let system = self
            .system_cpu_usage?
            .checked_sub(previous.system_cpu_usage?)?;
        if system == 0 {
            return None;
        }
        let percent = cpu as f64 / system as f64 * cores as f64 * 100.0;
        percent.is_finite().then_some(percent)
    }
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Memory {
    usage: Option<u64>,
    stats: BTreeMap<String, u64>,
}

impl Memory {
    fn bytes(&self) -> Option<u64> {
        let usage = self.usage?;
        let inactive = self
            .stats
            .get("total_inactive_file")
            .filter(|value| **value < usage)
            .or_else(|| {
                self.stats
                    .get("inactive_file")
                    .filter(|value| **value < usage)
            });
        Some(usage - inactive.copied().unwrap_or(0))
    }
}

#[derive(Deserialize)]
struct Sample {
    id: String,
    read: Option<String>,
    #[serde(default)]
    cpu_stats: Cpu,
    #[serde(default)]
    memory_stats: Memory,
}

impl Sample {
    fn read_time(&self) -> Option<OffsetDateTime> {
        OffsetDateTime::parse(self.read.as_deref()?, &Rfc3339)
            .ok()
            .filter(|value| value.unix_timestamp() > 0)
    }

    fn metrics_since(&self, previous: Option<&Self>) -> Metrics {
        let cpu_percent = previous.and_then(|previous| {
            (self.read_time()? > previous.read_time()?)
                .then(|| self.cpu_stats.percent_since(&previous.cpu_stats))
                .flatten()
        });
        Metrics {
            cpu_percent,
            memory_bytes: self.read_time().and_then(|_| self.memory_stats.bytes()),
        }
    }
}

fn samples(
    client: &Client<'_>,
    api: &str,
    ids: &[String],
    errors: &mut BTreeSet<String>,
) -> BTreeMap<String, Sample> {
    let paths: Vec<_> = ids
        .iter()
        .map(|id| format!("/{api}/containers/{id}/stats?stream=false&one-shot=true"))
        .collect();
    ids.iter()
        .zip(client.batch(&paths))
        .filter_map(|(id, response)| {
            match response.and_then(|response| response.json::<Sample>()) {
                Ok(sample) if sample.id == *id => Some((id.clone(), sample)),
                Ok(_) => {
                    errors.insert("Docker API sample identity mismatch".into());
                    None
                }
                Err(error) => {
                    errors.insert(error.to_string());
                    None
                }
            }
        })
        .collect()
}

pub(super) fn collect(
    client: &Client<'_>,
    ids: &[String],
    interval: Duration,
    requested_version: Option<&str>,
) -> Result<Option<Batch>> {
    if ids.is_empty() {
        return Ok(Some(Batch {
            metrics: BTreeMap::new(),
            memory_basis: API_MEMORY_BASIS,
            unavailable: Vec::new(),
        }));
    }
    if ids
        .iter()
        .any(|id| id.len() != 64 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        bail!("invalid Docker stats request identity");
    }
    let response = client.get("/version")?;
    if response.status == 404 {
        return Ok(None);
    }
    let server: Version = response.json()?;
    let Some(api) = negotiated(&server, requested_version) else {
        return Ok(None);
    };
    let mut errors = BTreeSet::new();
    let before = samples(client, &api, ids, &mut errors);
    client.wait(interval)?;
    let after = samples(client, &api, ids, &mut errors);
    client.check()?;
    Ok(Some(Batch {
        metrics: after
            .into_iter()
            .map(|(id, sample)| {
                let metrics = sample.metrics_since(before.get(&id));
                (id, metrics)
            })
            .collect(),
        memory_basis: API_MEMORY_BASIS,
        unavailable: errors
            .into_iter()
            .map(|message| Diagnostic::new("unavailable", "docker.stats", message))
            .collect(),
    }))
}

#[cfg(test)]
mod tests;
