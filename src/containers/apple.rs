mod xpc;

use super::{
    attribution,
    command::{self, Runner, RuntimeCommand, fail},
    model::{Container, Mount, Port, Runtime},
    stats::Batch,
};
use crate::{
    extensions::host::projects::Discovery,
    extensions::host::stable_ref,
    model::{Diagnostic, FailureReason},
    snapshot::Cancellation,
};
use anyhow::Result;
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    time::{Duration, Instant},
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

const MEMORY_BASIS: &str = "apple_xpc_memory_usage_bytes";
const MAX_CONTAINERS: usize = 256;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Configuration {
    id: String,
    resources: Option<Resources>,
    creation_date: Option<String>,
    #[serde(default)]
    labels: BTreeMap<String, String>,
    init_process: Option<Process>,
    mounts: Option<Vec<Filesystem>>,
    published_ports: Option<Vec<PublishedPort>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Resources {
    cpus: Option<u32>,
    memory_in_bytes: Option<u64>,
    cpu_overhead: Option<u32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Process {
    working_directory: Option<String>,
}

#[derive(Deserialize)]
struct Filesystem {
    #[serde(rename = "type")]
    kind: BTreeMap<String, serde_json::Value>,
    source: PathBuf,
    destination: PathBuf,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublishedPort {
    host_address: Option<String>,
    host_port: u16,
    container_port: u16,
    proto: String,
    #[serde(default = "one")]
    count: u16,
}
fn one() -> u16 {
    1
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    state: Option<String>,
    started_date: Option<String>,
}

#[derive(Deserialize)]
struct Item {
    configuration: Configuration,
    status: Status,
}

fn date(value: Option<&str>, now: OffsetDateTime) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(value?, &Rfc3339)
        .ok()
        .filter(|start| start.unix_timestamp() > 0 && *start <= now)
}

fn inventory(runner: &mut impl Runner) -> Result<Vec<Item>> {
    let result = runner.run(&[
        "list".into(),
        "--all".into(),
        "--format".into(),
        "json".into(),
    ])?;
    if result.issue.is_some() {
        return Err(
            result.failure("Apple Container inventory unavailable; CLI diagnostics withheld")
        );
    }
    let items: Vec<Item> = serde_json::from_slice(&result.stdout).map_err(|_| {
        fail(
            FailureReason::DaemonError,
            "invalid Apple Container inventory",
        )
    })?;
    let mut seen = BTreeSet::new();
    for item in &items {
        let id = &item.configuration.id;
        if id.is_empty() || id.len() > 1024 || id.chars().any(char::is_control) || !seen.insert(id)
        {
            return Err(fail(
                FailureReason::DaemonError,
                "invalid or duplicate Apple Container ID",
            ));
        }
    }
    Ok(items)
}

fn ports(values: Option<&[PublishedPort]>) -> Option<Vec<Port>> {
    let mut result = Vec::new();
    for port in values? {
        if port.count == 0 || result.len() + usize::from(port.count) > 4096 {
            return None;
        }
        for offset in 0..port.count {
            result.push(Port {
                protocol: port.proto.clone(),
                container_port: port.container_port.checked_add(offset)?,
                host_port: Some(port.host_port.checked_add(offset)?),
                host_ip: port.host_address.clone(),
            });
        }
    }
    Some(result)
}

fn apply(item: &Item, endpoint: &str, now: OffsetDateTime, discovery: &mut Discovery) -> Container {
    let config = &item.configuration;
    let mut container = Container {
        runtime: "apple".into(),
        host_scope: Some(format!("apple:{}", config.id)),
        configuration: config.resources.as_ref().map(|r| {
            Box::new(crate::model::ResourceConfiguration {
                scope: "container_limit".into(),
                cpus: r.cpus,
                memory_bytes: r.memory_in_bytes,
                cpu_overhead: r.cpu_overhead,
            })
        }),
        metrics_scope: Some("in_vm".into()),
        endpoint_ref: endpoint.into(),
        id: config.id.clone(),
        name: config.id.clone(),
        state: item.status.state.clone(),
        started_at: item.status.started_date.clone(),
        memory_basis: MEMORY_BASIS.into(),
        ports: ports(config.published_ports.as_deref()),
        mounts: config
            .mounts
            .iter()
            .flatten()
            .map(|mount| Mount {
                kind: if mount.kind.len() == 1 {
                    match mount.kind.keys().next().map(String::as_str) {
                        Some("virtiofs") => "bind",
                        Some(kind) => kind,
                        None => "unknown",
                    }
                } else {
                    "unknown"
                }
                .into(),
                source: mount.source.clone(),
                destination: mount.destination.clone(),
            })
            .collect(),
        service: config.labels.get("com.docker.compose.service").cloned(),
        ..Container::default()
    };
    if config.mounts.is_none() {
        container.unavailable.push(Diagnostic::new(
            "unavailable",
            "apple.inventory",
            "mount metadata unavailable",
        ));
    }
    if container.ports.is_none() {
        container.unavailable.push(Diagnostic::new(
            "invalid_data",
            "apple.inventory",
            "published ports unavailable or invalid",
        ));
    }
    if container.is_active() {
        container.uptime_seconds = date(container.started_at.as_deref(), now)
            .map(|start| (now - start).whole_seconds() as u64);
    }
    attribution::classify(
        &mut container,
        &attribution::Evidence {
            workdir: config
                .init_process
                .as_ref()
                .and_then(|process| process.working_directory.clone()),
            compose_project: config.labels.get("com.docker.compose.project").cloned(),
            compose_workdir: config
                .labels
                .get("com.docker.compose.project.working_dir")
                .cloned(),
        },
        true,
        discovery,
    );
    container
}

fn stable(before: &Item, after: &Item, now: OffsetDateTime) -> bool {
    before.configuration.id == after.configuration.id
        && before.status.state.as_deref() == Some("running")
        && before.status.state == after.status.state
        && before.configuration.creation_date == after.configuration.creation_date
        && date(before.configuration.creation_date.as_deref(), now).is_some()
        && before.status.started_date == after.status.started_date
        && date(before.status.started_date.as_deref(), now).is_some()
}

#[cfg(test)]
pub(super) fn collect(
    runner: &mut impl Runner,
    now: OffsetDateTime,
    stats: impl FnMut(&[String]) -> Result<Batch>,
) -> (Runtime, Vec<Container>) {
    collect_with_service(runner, now, stats, || None)
}

pub(super) const SERVICE_LABEL: &str = "com.apple.container.apiserver";
const LAUNCHCTL_SERVICE_NOT_FOUND: i32 = 113;

pub(super) fn collect_with_service(
    runner: &mut impl Runner,
    now: OffsetDateTime,
    mut stats: impl FnMut(&[String]) -> Result<Batch>,
    service: impl FnOnce() -> Option<i32>,
) -> (Runtime, Vec<Container>) {
    let start = Instant::now();
    let endpoint = stable_ref(
        "endpoint",
        &format!("apple\0{SERVICE_LABEL}\0{}", unsafe { libc::geteuid() }),
    );
    let mut runtime = Runtime {
        disk_usage: None,
        metrics_scope: Some("in_vm".into()),
        host_scope: None,
        configuration: None,
        name: "apple".into(),
        context: Some("local".into()),
        endpoint_ref: Some(endpoint.clone()),
        daemon_id: None,
        status: "unavailable".into(),
        inventory_complete: false,
        collection_ms: 0,
        unavailable: vec![],
    };
    let result = (|| -> Result<Vec<Container>> {
        let version = runner.run(&["--version".into()])?;
        let supported = version.issue.is_none()
            && String::from_utf8_lossy(&version.stdout)
                .split_whitespace()
                .any(|word| word == "1.6.1");
        let before = inventory(runner)?;
        runtime.status = "available".into();
        runtime.inventory_complete = before.len() <= MAX_CONTAINERS;
        if !runtime.inventory_complete {
            runtime.unavailable.push(Diagnostic::new(
                "limit_exceeded",
                "apple.inventory",
                "inventory truncated at 256 containers",
            ));
        }
        let before: Vec<_> = before.into_iter().take(MAX_CONTAINERS).collect();
        let ids: Vec<_> = before
            .iter()
            .filter(|item| item.status.state.as_deref() == Some("running"))
            .map(|item| item.configuration.id.clone())
            .collect();
        let batch = if !ids.is_empty() && supported {
            match stats(&ids) {
                Ok(batch) => {
                    runtime.unavailable.extend(batch.unavailable.clone());
                    Some(batch)
                }
                Err(_) => {
                    runtime.unavailable.push(Diagnostic::new(
                        "unavailable",
                        "apple.stats",
                        "XPC stats collection failed",
                    ));
                    None
                }
            }
        } else {
            if !ids.is_empty() && !supported {
                runtime.unavailable.push(Diagnostic::new(
                    "unsupported",
                    "apple.stats",
                    "XPC stats require Apple Container 1.6.1; inventory remains available",
                ));
            }
            None
        };
        let after = if batch.is_some() {
            match inventory(runner) {
                Ok(items) => items
                    .into_iter()
                    .map(|item| (item.configuration.id.clone(), item))
                    .collect(),
                Err(_) => {
                    runtime.unavailable.push(Diagnostic::new(
                        "unavailable",
                        "apple.inventory",
                        "post-stats inventory unavailable",
                    ));
                    BTreeMap::new()
                }
            }
        } else {
            BTreeMap::new()
        };
        let mut discovery = Discovery::default();
        let mut containers = Vec::new();
        for original in &before {
            let fresh = after.get(&original.configuration.id);
            let mut container = apply(fresh.unwrap_or(original), &endpoint, now, &mut discovery);
            if let Some(batch) = &batch {
                if fresh.is_some_and(|fresh| stable(original, fresh, now)) {
                    if let Some(metrics) = batch.metrics.get(&container.id) {
                        container.cpu_percent = metrics.cpu_percent;
                        container.memory_bytes = metrics.memory_bytes;
                    }
                } else if original.status.state.as_deref() == Some("running") {
                    container.unavailable.push(Diagnostic::new(
                        "identity_unverified",
                        "apple.stats",
                        "state/start/creation changed or could not be revalidated during stats",
                    ));
                }
            }
            if !container.is_inactive()
                && (container.cpu_percent.is_none() || container.memory_bytes.is_none())
            {
                container.unavailable.push(Diagnostic::new(
                    "unavailable",
                    "apple.stats",
                    "stats unavailable or invalid",
                ));
            }
            containers.push(container);
        }
        Ok(containers)
    })();
    let mut probe = command::Probe::new(runner);
    let containers = result.unwrap_or_else(|error| {
        let mut diagnostic = probe.diagnose("collection_failed", "apple", &error, None);
        if diagnostic.reason == Some(FailureReason::Unknown)
            && !probe.canceled()
            && !probe.expired()
            && service() == Some(LAUNCHCTL_SERVICE_NOT_FOUND)
        {
            diagnostic.refine(FailureReason::EndpointNotFound, None);
        }
        runtime.unavailable.push(diagnostic);
        vec![]
    });
    runtime.collection_ms = start.elapsed().as_millis() as u64;
    (runtime, containers)
}

pub(super) fn native(
    program: PathBuf,
    cancel: &Cancellation,
    deadline: Instant,
    interval: Duration,
) -> (Runtime, Vec<Container>) {
    let mut runner = RuntimeCommand {
        program,
        cancel: cancel.clone(),
        deadline,
        sample_interval: interval,
        api_version: None,
    };
    collect_with_service(
        &mut runner,
        OffsetDateTime::now_utc(),
        |ids| xpc::collect(ids, cancel, deadline, interval),
        || {
            super::command::run_bounded(
                std::path::Path::new("/bin/launchctl"),
                &[
                    "print".into(),
                    format!("gui/{}/{SERVICE_LABEL}", unsafe { libc::geteuid() }),
                ],
                None,
                cancel,
                deadline,
            )
            .ok()
            .filter(|output| !output.timed_out)
            .and_then(|output| output.exit_code)
        },
    )
}

#[cfg(test)]
mod tests;

pub(crate) fn vm_disk(path: &std::path::Path, home: &std::path::Path) -> Option<String> {
    let root = home.join("Library/Application Support/com.apple.container/containers");
    let parts: Vec<_> = path.strip_prefix(root).ok()?.components().collect();
    if let [
        std::path::Component::Normal(id),
        std::path::Component::Normal(disk),
    ] = parts.as_slice()
        && *disk == "rootfs.ext4"
        && id.to_str().is_some_and(|s| {
            !s.is_empty()
                && s.len() <= 1024
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        })
    {
        return Some(format!("apple:{}", id.to_str()?));
    }
    None
}

pub(crate) fn vm_helper(
    executable: &std::path::Path,
) -> Option<(std::path::PathBuf, Option<std::path::PathBuf>, String)> {
    let root = std::path::Path::new("/usr/local/libexec/container/plugins");
    let parts: Vec<_> = executable.strip_prefix(root).ok()?.components().collect();
    if parts
        .iter()
        .any(|p| !matches!(p, std::path::Component::Normal(_)))
    {
        return None;
    }
    if executable == root.join("container-runtime-linux/bin/container-runtime-linux") {
        Some((executable.to_path_buf(), None, "Apple Container".into()))
    } else {
        None
    }
}
