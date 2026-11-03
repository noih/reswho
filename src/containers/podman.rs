use super::{
    Container, Runtime,
    command::{self, Runner},
    docker::{self, Inspect},
};
use crate::{
    model::Diagnostic,
    snapshot::Cancellation,
    tree::{projects::Discovery, stable_ref},
};
use anyhow::{Result, bail};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    time::{Duration, Instant},
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

const MAX_CONTAINERS: usize = 256;
const BATCH_SIZE: usize = 32;
const MEMORY_BASIS: &str = "podman_cgroup_usage_bytes";
const INFO_FORMAT: &str = r#"{"version":{{json .Version.Version}},"os":{{json .Host.OS}},"remote":{{json .Host.ServiceIsRemote}},"hostname":{{json .Host.Hostname}},"graphroot":{{json .Store.GraphRoot}}}"#;
const STATS_FORMAT: &str = r#"{"id":{{json .ContainerID}},"cpu_nano":{{json .CPUNano}},"system_nano":{{json .SystemNano}},"memory":{{json .ContainerStats.MemUsage}},"pids":{{json .PIDs}}}"#;

#[derive(Deserialize)]
struct Info {
    version: Option<String>,
    os: String,
    remote: bool,
    hostname: String,
    graphroot: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Connection {
    name: String,
    #[serde(rename = "URI")]
    uri: String,
    default: bool,
}
#[derive(Clone, Deserialize)]
struct Counter {
    id: String,
    cpu_nano: Option<u64>,
    system_nano: Option<u64>,
    memory: Option<u64>,
    pids: Option<u64>,
}

fn issue(operation: &str, code: &'static str) -> Diagnostic {
    Diagnostic::new(
        code,
        format!("podman.{operation}"),
        format!("Podman {operation} unavailable or invalid; raw diagnostics withheld"),
    )
}
fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| (*arg).into()).collect()
}
fn required(runner: &mut impl Runner, args: &[String]) -> Result<Vec<u8>> {
    let output = runner.run(args)?;
    if output.issue.is_some() {
        bail!("Podman CLI unavailable; diagnostics withheld");
    }
    Ok(output.stdout)
}
fn valid_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|byte| byte.is_ascii_hexdigit())
}
fn parse_lines<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<Vec<T>> {
    std::str::from_utf8(bytes)?
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| Ok(serde_json::from_str(line)?))
        .collect()
}
fn source(
    runner: &mut impl Runner,
    info: &Info,
    connection: Option<&str>,
    host: Option<&str>,
) -> Result<(Vec<String>, String)> {
    if !info.remote {
        return Ok((
            vec![],
            format!(
                "local\0{}\0{}\0{}",
                info.hostname,
                unsafe { libc::geteuid() },
                info.graphroot
            ),
        ));
    }
    if connection.is_none()
        && let Some(host) = host
    {
        if host.is_empty() {
            bail!("empty Podman endpoint");
        }
        return Ok((vec![format!("--url={host}")], host.into()));
    }
    let connections: Vec<Connection> = serde_json::from_slice(&required(
        runner,
        &strings(&["system", "connection", "list", "--format", "json"]),
    )?)?;
    let selected: Vec<_> = connections
        .iter()
        .filter(|candidate| connection.map_or(candidate.default, |name| name == candidate.name))
        .collect();
    let [selected] = selected.as_slice() else {
        bail!("Podman connection unresolved");
    };
    if selected.uri.is_empty() || selected.name.is_empty() {
        bail!("Podman connection unresolved");
    }
    Ok((
        vec![format!("--connection={}", selected.name)],
        selected.uri.clone(),
    ))
}
fn request(
    runner: &mut impl Runner,
    prefix: &[String],
    args: &[&str],
    ids: &[String],
) -> Result<Vec<u8>> {
    let mut command = prefix.to_vec();
    command.extend(strings(args));
    if !ids.is_empty() {
        command.push("--".into());
        command.extend_from_slice(ids);
    }
    required(runner, &command)
}
fn inspections(
    runner: &mut impl Runner,
    prefix: &[String],
    ids: &[String],
    diagnostics: &mut Vec<Diagnostic>,
) -> BTreeMap<String, Inspect> {
    let mut result = BTreeMap::new();
    let format = docker::INSPECT_FORMAT.replace("json .Id", "json .ID");
    for chunk in ids.chunks(BATCH_SIZE) {
        let batch = request(
            runner,
            prefix,
            &["container", "inspect", "--format", &format],
            chunk,
        )
        .and_then(|bytes| parse_lines::<Inspect>(&bytes))
        .and_then(|items| {
            let mut seen = BTreeSet::new();
            if items
                .iter()
                .any(|item| !chunk.contains(&item.id) || !seen.insert(&item.id))
            {
                bail!("invalid inspect identity");
            }
            Ok(items)
        });
        match batch {
            Ok(items) => result.extend(items.into_iter().map(|item| (item.id.clone(), item))),
            Err(_) => diagnostics.push(issue("inspect", "unavailable")),
        }
    }
    result
}
fn counters(
    runner: &mut impl Runner,
    prefix: &[String],
    ids: &[String],
    diagnostics: &mut Vec<Diagnostic>,
) -> BTreeMap<String, Counter> {
    let mut result = BTreeMap::new();
    for chunk in ids.chunks(BATCH_SIZE) {
        let batch = request(
            runner,
            prefix,
            &[
                "stats",
                "--no-stream",
                "--no-trunc",
                "--format",
                STATS_FORMAT,
            ],
            chunk,
        )
        .and_then(|bytes| parse_lines::<Counter>(&bytes))
        .and_then(|items| {
            let mut seen = BTreeSet::new();
            if items
                .iter()
                .any(|item| !chunk.contains(&item.id) || !seen.insert(&item.id))
            {
                bail!("invalid stats identity");
            }
            Ok(items)
        });
        match batch {
            Ok(items) => result.extend(items.into_iter().map(|item| (item.id.clone(), item))),
            Err(_) => diagnostics.push(issue("stats", "unsupported_or_unavailable")),
        }
    }
    result
}
fn metrics(before: &Counter, after: &Counter) -> (Option<f64>, Option<u64>) {
    if !before.pids.is_some_and(|pids| pids > 0) || !after.pids.is_some_and(|pids| pids > 0) {
        return (None, None);
    }
    let cpu = (|| {
        let elapsed = after.system_nano?.checked_sub(before.system_nano?)?;
        let used = after.cpu_nano?.checked_sub(before.cpu_nano?)?;
        (elapsed > 0).then_some(used as f64 / elapsed as f64 * 100.0)
    })();
    (cpu, after.memory)
}
fn stable(before: &Inspect, after: &Inspect, now: OffsetDateTime) -> bool {
    before.id == after.id
        && before.state == after.state
        && matches!(before.state.as_deref(), Some("running" | "paused"))
        && before.started_at == after.started_at
        && before.started_at.as_deref().is_some_and(|value| {
            OffsetDateTime::parse(value, &Rfc3339)
                .is_ok_and(|start| start.unix_timestamp() > 0 && start <= now)
        })
}
fn empty_runtime() -> Runtime {
    Runtime {
        name: "podman".into(),
        context: Some("unresolved".into()),
        endpoint_ref: Some(stable_ref("endpoint", "podman\0unresolved")),
        daemon_id: None,
        status: "unavailable".into(),
        inventory_complete: false,
        collection_ms: 0,
        unavailable: vec![],
    }
}
fn collect(
    runner: &mut impl Runner,
    cancel: &Cancellation,
    deadline: Instant,
    interval: Duration,
    selection: (Option<&str>, Option<&str>),
    now: OffsetDateTime,
) -> (Runtime, Vec<Container>) {
    let started = Instant::now();
    let mut runtime = empty_runtime();
    let result = (|| -> Result<Vec<Container>> {
        let info: Info = serde_json::from_slice(&required(
            runner,
            &strings(&["info", "--format", INFO_FORMAT]),
        )?)?;
        if info.hostname.is_empty() || info.graphroot.is_empty() {
            bail!("Podman server identity unavailable");
        }
        let (prefix, endpoint) = source(runner, &info, selection.0, selection.1)?;
        runtime.context = Some(if info.remote { "remote" } else { "local" }.into());
        runtime.endpoint_ref = Some(stable_ref(
            "endpoint",
            &format!("podman\0{endpoint}\0{}\0{}", info.hostname, info.graphroot),
        ));
        let bytes = request(
            runner,
            &prefix,
            &["container", "ls", "--all", "--quiet", "--no-trunc"],
            &[],
        )?;
        let mut ids = Vec::new();
        let mut seen = BTreeSet::new();
        for id in std::str::from_utf8(&bytes)?
            .lines()
            .filter(|line| !line.is_empty())
        {
            if !valid_id(id) || !seen.insert(id) {
                bail!("invalid Podman identity");
            }
            ids.push(id.to_owned());
        }
        runtime.status = "available".into();
        runtime.inventory_complete = ids.len() <= MAX_CONTAINERS;
        if !runtime.inventory_complete {
            runtime
                .unavailable
                .push(issue("inventory", "limit_exceeded"));
        }
        ids.truncate(MAX_CONTAINERS);
        let before = inspections(runner, &prefix, &ids, &mut runtime.unavailable);
        let active: Vec<_> = ids
            .iter()
            .filter(|id| {
                before
                    .get(*id)
                    .is_some_and(|item| matches!(item.state.as_deref(), Some("running" | "paused")))
            })
            .cloned()
            .collect();
        let supported = info.os == "linux"
            && info
                .version
                .as_deref()
                .is_some_and(|version| version.parse::<node_semver::Version>().is_ok());
        let (first, second) = if !active.is_empty() && supported {
            let first = counters(runner, &prefix, &active, &mut runtime.unavailable);
            if !first.is_empty() {
                let wake = Instant::now() + interval;
                while Instant::now() < wake {
                    cancel.check()?;
                    if Instant::now() >= deadline {
                        break;
                    }
                    std::thread::sleep(
                        wake.saturating_duration_since(Instant::now())
                            .min(Duration::from_millis(10)),
                    );
                }
                let second = counters(runner, &prefix, &active, &mut runtime.unavailable);
                (first, second)
            } else {
                (first, BTreeMap::new())
            }
        } else {
            if !active.is_empty() {
                runtime.unavailable.push(issue("stats", "unsupported"));
            }
            (BTreeMap::new(), BTreeMap::new())
        };
        let after = if !first.is_empty() {
            inspections(runner, &prefix, &active, &mut runtime.unavailable)
        } else {
            BTreeMap::new()
        };
        let mut discovery = Discovery::default();
        Ok(ids
            .into_iter()
            .map(|id| {
                let mut container = Container {
                    runtime: "podman".into(),
                    endpoint_ref: runtime.endpoint_ref.clone().unwrap_or_default(),
                    name: id[..12].into(),
                    id: id.clone(),
                    memory_basis: MEMORY_BASIS.into(),
                    ..Container::default()
                };
                if let Some(item) = after.get(&id).or_else(|| before.get(&id)) {
                    docker::apply(&mut container, item.clone(), now, false, &mut discovery);
                    if container.state.as_deref() == Some("initialized") {
                        container.state = Some("created".into());
                    }
                    if let (Some(original), Some(fresh), Some(a), Some(b)) = (
                        before.get(&id),
                        after.get(&id),
                        first.get(&id),
                        second.get(&id),
                    ) {
                        if stable(original, fresh, now) {
                            (container.cpu_percent, container.memory_bytes) = metrics(a, b);
                        } else {
                            container
                                .unavailable
                                .push(issue("stats", "identity_unverified"));
                        }
                    }
                } else {
                    container.unavailable.push(issue("inspect", "unavailable"));
                }
                if !container.is_inactive()
                    && (container.cpu_percent.is_none() || container.memory_bytes.is_none())
                {
                    container.unavailable.push(issue("stats", "unavailable"));
                }
                container
            })
            .collect())
    })();
    let containers = result.unwrap_or_else(|_| {
        runtime.unavailable.push(issue(
            "collect",
            if cancel.check().is_err() {
                "canceled"
            } else if Instant::now() >= deadline {
                "timeout"
            } else {
                "unavailable"
            },
        ));
        vec![]
    });
    runtime.collection_ms = started.elapsed().as_millis() as u64;
    (runtime, containers)
}

struct PodmanCommand<'a> {
    program: PathBuf,
    cancel: &'a Cancellation,
    deadline: Instant,
}
impl Runner for PodmanCommand<'_> {
    fn run(&mut self, args: &[String]) -> Result<command::Output> {
        command::run_bounded(&self.program, args, Some(b""), self.cancel, self.deadline)
    }
}

pub(super) fn native(
    cancel: &Cancellation,
    deadline: Instant,
    interval: Duration,
) -> (Runtime, Vec<Container>) {
    let Some(program) = command::find_program("podman") else {
        let mut runtime = empty_runtime();
        runtime.unavailable.push(issue("cli", "not_found"));
        return (runtime, vec![]);
    };
    let mut runner = PodmanCommand {
        program,
        cancel,
        deadline,
    };
    let connection = std::env::var("CONTAINER_CONNECTION")
        .ok()
        .filter(|value| !value.is_empty());
    let host = std::env::var("CONTAINER_HOST")
        .ok()
        .filter(|value| !value.is_empty());
    collect(
        &mut runner,
        cancel,
        deadline,
        interval,
        (connection.as_deref(), host.as_deref()),
        OffsetDateTime::now_utc(),
    )
}

#[cfg(test)]
mod tests;
