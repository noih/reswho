use super::{
    attribution,
    command::{Output, Runner},
    model::{Container, Mount, Port, Runtime},
    stats::{API_MEMORY_BASIS, Batch, CLI_MEMORY_BASIS, Metrics},
};
use crate::model::Diagnostic;
use crate::tree::{projects::Discovery, stable_ref};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Instant,
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

const MAX_CONTAINERS: usize = 256;
const BATCH_SIZE: usize = 32;
const INSPECT_FORMAT: &str = r#"{"id":{{json .Id}},"name":{{json .Name}},"state":{{json .State.Status}},"started_at":{{json .State.StartedAt}},"workdir":{{json .Config.WorkingDir}},"compose_project":{{json (index .Config.Labels "com.docker.compose.project")}},"compose_workdir":{{json (index .Config.Labels "com.docker.compose.project.working_dir")}},"service":{{json (index .Config.Labels "com.docker.compose.service")}},"network_mode":{{json .HostConfig.NetworkMode}},"ports":{{json .NetworkSettings.Ports}},"mounts":[{{range $i, $m := .Mounts}}{{if $i}},{{end}}{"kind":{{json $m.Type}},"source":{{json $m.Source}},"destination":{{json $m.Destination}}}{{end}}]}"#;
const STATS_FORMAT: &str =
    r#"{"id":{{json .ID}},"cpu":{{json .CPUPerc}},"memory":{{json .MemUsage}}}"#;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct Inspect {
    pub id: String,
    pub name: Option<String>,
    pub state: Option<String>,
    pub started_at: Option<String>,
    pub workdir: Option<String>,
    pub compose_project: Option<String>,
    pub compose_workdir: Option<String>,
    pub service: Option<String>,
    pub network_mode: Option<String>,
    ports: Option<BTreeMap<String, Option<Vec<Binding>>>>,
    pub mounts: Vec<Mount>,
}

#[derive(Debug, Deserialize)]
struct Binding {
    #[serde(rename = "HostIp")]
    host_ip: String,
    #[serde(rename = "HostPort")]
    host_port: String,
}

#[derive(Deserialize)]
struct Stats {
    id: String,
    cpu: String,
    memory: String,
}

#[derive(Deserialize)]
struct Info {
    id: String,
    #[serde(default)]
    os: String,
}

struct Source {
    args: Vec<String>,
    host: String,
    context: Option<String>,
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|value| (*value).into()).collect()
}

fn required(runner: &mut impl Runner, args: &[String]) -> Result<String> {
    let output = runner.run(args)?;
    if let Some(issue) = output.issue {
        bail!("{issue}");
    }
    String::from_utf8(output.stdout).context("Docker returned invalid UTF-8")
}

fn source(
    runner: &mut impl Runner,
    context: Option<String>,
    host: Option<String>,
) -> Result<Source> {
    let context = context.filter(|value| !value.is_empty());
    if context.is_none()
        && let Some(host) = host.filter(|value| !value.is_empty())
    {
        return Ok(Source {
            args: vec![format!("--host={host}")],
            host,
            context: None,
        });
    }
    let context = match context {
        Some(context) => context,
        None => required(runner, &strings(&["context", "show"]))?
            .trim()
            .to_owned(),
    };
    if context.is_empty() {
        bail!("Docker context is empty");
    }
    let mut args = strings(&[
        "context",
        "inspect",
        "--format",
        "{{json .Endpoints.docker.Host}}",
        "--",
    ]);
    args.push(context.clone());
    let host: String = serde_json::from_str(required(runner, &args)?.trim())
        .context("Docker context endpoint unavailable")?;
    if host.is_empty() {
        bail!("Docker endpoint is empty");
    }
    Ok(Source {
        args: vec![format!("--context={context}")],
        host,
        context: Some(context),
    })
}

fn request(
    runner: &mut impl Runner,
    source: &Source,
    args: &[&str],
    ids: &[String],
) -> Result<Output> {
    let mut command = source.args.clone();
    command.extend(strings(args));
    if !ids.is_empty() {
        command.push("--".into());
        command.extend_from_slice(ids);
    }
    runner.run(&command)
}

fn valid_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn inventory(
    runner: &mut impl Runner,
    source: &Source,
    runtime: &mut Runtime,
) -> Result<Vec<String>> {
    let output = request(
        runner,
        source,
        &["container", "ls", "--all", "--quiet", "--no-trunc"],
        &[],
    )?;
    if let Some(issue) = output.issue {
        bail!("inventory: {issue}");
    }
    let text = String::from_utf8(output.stdout).context("invalid Docker inventory encoding")?;
    let mut ids = BTreeSet::new();
    for id in text.lines().map(str::trim).filter(|id| !id.is_empty()) {
        if !valid_id(id) || !ids.insert(id.to_owned()) {
            bail!("invalid or duplicate Docker container ID");
        }
    }
    runtime.inventory_complete = ids.len() <= MAX_CONTAINERS;
    if !runtime.inventory_complete {
        runtime.unavailable.push(Diagnostic::new(
            "limit_exceeded",
            "docker.inventory",
            "inventory truncated at 256 containers",
        ));
    }
    Ok(ids.into_iter().take(MAX_CONTAINERS).collect())
}

fn inspections(
    runner: &mut impl Runner,
    source: &Source,
    ids: &[String],
    issues: &mut Vec<Diagnostic>,
) -> BTreeMap<String, Inspect> {
    let mut result = BTreeMap::new();
    for batch in ids.chunks(BATCH_SIZE) {
        let output = match request(
            runner,
            source,
            &["container", "inspect", "--format", INSPECT_FORMAT],
            batch,
        ) {
            Ok(output) => output,
            Err(error) => {
                issues.push(Diagnostic::new(
                    "command_failed",
                    "docker.inspect",
                    format!("inspect: {error}"),
                ));
                break;
            }
        };
        if let Some(issue) = output.issue {
            issues.push(Diagnostic::new(
                "command_failed",
                "docker.inspect",
                format!("inspect: {issue}"),
            ));
        }
        for line in output
            .stdout
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
        {
            match serde_json::from_slice::<Inspect>(line) {
                Ok(inspect) if batch.contains(&inspect.id) && !result.contains_key(&inspect.id) => {
                    result.insert(inspect.id.clone(), inspect);
                }
                _ => issues.push(Diagnostic::new(
                    "invalid_data",
                    "docker.inspect",
                    "inspect: invalid, duplicate or unexpected container record",
                )),
            }
        }
    }
    result
}

fn number(value: &str) -> Option<f64> {
    value
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value >= 0.0)
}

fn memory_bytes(value: &str) -> Option<u64> {
    let used = value.split('/').next()?.trim();
    let boundary = used.find(|ch: char| ch.is_alphabetic())?;
    let amount = number(&used[..boundary])?;
    let scale = match used[boundary..].trim() {
        "B" => 1.0,
        "KiB" => 1024.0,
        "MiB" => 1024.0f64.powi(2),
        "GiB" => 1024.0f64.powi(3),
        "TiB" => 1024.0f64.powi(4),
        "PiB" => 1024.0f64.powi(5),
        _ => return None,
    };
    let bytes = amount * scale;
    (bytes.is_finite() && bytes < u64::MAX as f64).then_some(bytes.round() as u64)
}

fn stats(
    runner: &mut impl Runner,
    source: &Source,
    ids: &[String],
    issues: &mut Vec<Diagnostic>,
) -> Batch {
    match runner.direct_stats(&source.host, ids) {
        Ok(Some(batch)) => return batch,
        Ok(None) => {}
        Err(error) => {
            return Batch {
                metrics: BTreeMap::new(),
                memory_basis: API_MEMORY_BASIS,
                unavailable: vec![Diagnostic::new(
                    "unavailable",
                    "docker.stats",
                    error.to_string(),
                )],
            };
        }
    }
    let mut result = BTreeMap::new();
    for batch in ids.chunks(BATCH_SIZE) {
        let output = match request(
            runner,
            source,
            &[
                "container",
                "stats",
                "--no-stream",
                "--no-trunc",
                "--format",
                STATS_FORMAT,
            ],
            batch,
        ) {
            Ok(output) => output,
            Err(error) => {
                issues.push(Diagnostic::new(
                    "command_failed",
                    "docker.stats",
                    format!("stats: {error}"),
                ));
                break;
            }
        };
        if let Some(issue) = output.issue {
            issues.push(Diagnostic::new(
                "command_failed",
                "docker.stats",
                format!("stats: {issue}"),
            ));
            continue;
        }
        let mut parsed = BTreeMap::new();
        let mut invalid = false;
        for line in output
            .stdout
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
        {
            match serde_json::from_slice::<Stats>(line) {
                Ok(stat) if batch.contains(&stat.id) && !parsed.contains_key(&stat.id) => {
                    parsed.insert(
                        stat.id,
                        Metrics {
                            cpu_percent: stat.cpu.strip_suffix('%').and_then(number),
                            memory_bytes: memory_bytes(&stat.memory),
                        },
                    );
                }
                _ => invalid = true,
            }
        }
        if invalid {
            issues.push(Diagnostic::new(
                "invalid_data",
                "docker.stats",
                "stats: invalid, duplicate or unexpected container record",
            ));
        } else {
            result.extend(parsed);
        }
    }
    Batch {
        metrics: result,
        memory_basis: CLI_MEMORY_BASIS,
        unavailable: Vec::new(),
    }
}

fn ports(inspect: &Inspect) -> Option<Vec<Port>> {
    if inspect.network_mode.as_deref() == Some("host") {
        return None;
    }
    let mut result = Vec::new();
    for (port, bindings) in inspect.ports.as_ref()? {
        let (port, protocol) = port.split_once('/')?;
        let port: u16 = port.parse().ok().filter(|port| *port > 0)?;
        if !matches!(protocol, "tcp" | "udp" | "sctp") {
            return None;
        }
        if let Some(bindings) = bindings.as_ref().filter(|bindings| !bindings.is_empty()) {
            for binding in bindings {
                let host_port = binding
                    .host_port
                    .parse::<u16>()
                    .ok()
                    .filter(|port| *port > 0)?;
                result.push(Port {
                    protocol: protocol.into(),
                    container_port: port,
                    host_ip: Some(binding.host_ip.clone()),
                    host_port: Some(host_port),
                });
            }
        } else {
            result.push(Port {
                protocol: protocol.into(),
                container_port: port,
                host_ip: None,
                host_port: None,
            });
        }
    }
    Some(result)
}

fn active(state: Option<&str>) -> bool {
    matches!(state, Some("running" | "paused"))
}

fn apply(
    container: &mut Container,
    inspect: Inspect,
    now: OffsetDateTime,
    local: bool,
    discovery: &mut Discovery,
) {
    container.name = inspect
        .name
        .as_deref()
        .filter(|name| !name.trim_matches('/').is_empty())
        .map(|name| name.trim_start_matches('/').to_owned())
        .unwrap_or_else(|| container.id[..12].into());
    container.state = inspect.state.clone();
    container.started_at = inspect.started_at.clone();
    if active(inspect.state.as_deref()) {
        container.uptime_seconds = inspect
            .started_at
            .as_deref()
            .and_then(|value| OffsetDateTime::parse(value, &Rfc3339).ok())
            .filter(|start| start.unix_timestamp() > 0 && *start <= now)
            .map(|start| (now - start).whole_seconds() as u64);
    }
    container.ports = ports(&inspect);
    container.mounts = inspect.mounts.clone();
    container.service = inspect.service.clone().filter(|value| !value.is_empty());
    attribution::classify(
        container,
        &attribution::Evidence {
            workdir: inspect.workdir,
            compose_project: inspect.compose_project,
            compose_workdir: inspect.compose_workdir,
        },
        local,
        discovery,
    );
}

pub(super) fn collect(
    runner: &mut impl Runner,
    context: Option<String>,
    host: Option<String>,
    now: OffsetDateTime,
) -> (Runtime, Vec<Container>) {
    let started = Instant::now();
    let mut runtime = Runtime {
        name: "docker".into(),
        context: None,
        endpoint_ref: None,
        daemon_id: None,
        status: "unavailable".into(),
        inventory_complete: false,
        collection_ms: 0,
        unavailable: Vec::new(),
    };
    let result = collect_inner(runner, context, host, now, &mut runtime);
    let containers = match result {
        Ok(containers) => containers,
        Err(error) => {
            runtime.unavailable.push(Diagnostic::new(
                "collection_failed",
                "docker",
                error.to_string(),
            ));
            Vec::new()
        }
    };
    runtime.collection_ms = started.elapsed().as_millis() as u64;
    runtime
        .unavailable
        .sort_by(|a, b| a.message.cmp(&b.message));
    runtime.unavailable.dedup();
    (runtime, containers)
}

fn collect_inner(
    runner: &mut impl Runner,
    context: Option<String>,
    host: Option<String>,
    now: OffsetDateTime,
    runtime: &mut Runtime,
) -> Result<Vec<Container>> {
    let source = source(runner, context, host)?;
    runtime.context = source.context.clone();
    runtime.endpoint_ref = Some(stable_ref("endpoint", &format!("docker\0{}", source.host)));
    let info = request(
        runner,
        &source,
        &[
            "info",
            "--format",
            r#"{"id":{{json .ID}},"os":{{json .OSType}}}"#,
        ],
        &[],
    )?;
    if let Some(issue) = info.issue {
        bail!("info: {issue}");
    }
    let info: Info =
        serde_json::from_slice(&info.stdout).context("invalid Docker daemon metadata")?;
    if info.id.is_empty() {
        bail!("Docker daemon identity unavailable");
    }
    runtime.daemon_id = Some(info.id);
    let ids = inventory(runner, &source, runtime)?;
    runtime.status = "available".into();
    let before = inspections(runner, &source, &ids, &mut runtime.unavailable);
    let active_ids: Vec<_> = ids
        .iter()
        .filter(|id| {
            before
                .get(*id)
                .is_some_and(|item| active(item.state.as_deref()))
        })
        .cloned()
        .collect();
    let batch = if info.os == "linux" {
        stats(runner, &source, &active_ids, &mut runtime.unavailable)
    } else {
        runtime.unavailable.push(Diagnostic::new(
            "unsupported",
            "docker.stats",
            "container metrics require a Linux Docker daemon",
        ));
        Batch {
            metrics: BTreeMap::new(),
            memory_basis: CLI_MEMORY_BASIS,
            unavailable: Vec::new(),
        }
    };
    runtime.unavailable.extend(batch.unavailable);
    let metrics = batch.metrics;
    let after = if !metrics.is_empty() {
        inspections(runner, &source, &active_ids, &mut runtime.unavailable)
    } else {
        BTreeMap::new()
    };
    let mut discovery = Discovery::default();
    let mut containers = Vec::new();
    for id in ids {
        let mut container = Container {
            runtime: "docker".into(),
            endpoint_ref: runtime.endpoint_ref.clone().unwrap_or_default(),
            name: id[..12].into(),
            id: id.clone(),
            memory_basis: batch.memory_basis.into(),
            ..Container::default()
        };
        if let Some(inspect) = before.get(&id)
            && let Some(metric) = metrics.get(&id)
        {
            let stable = after.get(&id).is_some_and(|fresh| {
                fresh.state == inspect.state
                    && fresh.started_at == inspect.started_at
                    && fresh.started_at.as_deref().is_some_and(|value| {
                        OffsetDateTime::parse(value, &Rfc3339)
                            .is_ok_and(|start| start.unix_timestamp() > 0 && start <= now)
                    })
            });
            if stable {
                container.cpu_percent = metric.cpu_percent;
                container.memory_bytes = metric.memory_bytes;
            } else {
                container.unavailable.push(Diagnostic::new(
                    "identity_unverified",
                    "docker.stats",
                    "state/start changed or could not be revalidated during stats",
                ));
            }
        }
        containers.push(container);
    }
    let mut before = before;
    let mut after = after;
    for container in &mut containers {
        if let Some(inspect) = after
            .remove(&container.id)
            .or_else(|| before.remove(&container.id))
        {
            apply(
                container,
                inspect,
                now,
                source.host.starts_with("unix://"),
                &mut discovery,
            );
            if active(container.state.as_deref())
                && (container.cpu_percent.is_none() || container.memory_bytes.is_none())
            {
                container.unavailable.push(Diagnostic::new(
                    "unavailable",
                    "docker.stats",
                    "stats unavailable or invalid",
                ));
            }
        } else {
            container.unavailable.push(Diagnostic::new(
                "unavailable",
                "docker.inspect",
                "inspect unavailable",
            ));
        }
    }
    let missing_stats = containers
        .iter()
        .filter(|item| {
            active(item.state.as_deref())
                && (item.cpu_percent.is_none() || item.memory_bytes.is_none())
        })
        .count();
    if missing_stats > 0 {
        runtime.unavailable.push(Diagnostic::new(
            "unavailable",
            "docker.stats",
            format!("stats unavailable for {missing_stats} containers"),
        ));
    }
    let missing_details = containers
        .iter()
        .filter(|item| item.state.is_none())
        .count();
    if missing_details > 0 {
        runtime.unavailable.push(Diagnostic::new(
            "unavailable",
            "docker.inspect",
            format!("inspect/state unavailable for {missing_details} containers"),
        ));
    }
    Ok(containers)
}

#[cfg(test)]
mod tests;
