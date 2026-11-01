use super::*;
use crate::containers::{Container, ContainerTotals, ProjectSource};

pub(super) fn totals(reference: &str, totals: &ContainerTotals, collapsed: &str) -> String {
    let scope_value = |value: String| {
        if totals.inventory_complete || value == "?" || value.starts_with(">=") {
            value
        } else {
            format!(">={value}")
        }
    };
    if totals.containers == 0 && !totals.inventory_complete {
        return format!("{reference} · ? · ? · oldest ? · ? active / ? containers{collapsed}");
    }
    let applicable = totals.containers - totals.inactive_containers;
    let usage = |value: String, unknown: usize| {
        if applicable == 0 {
            if totals.inventory_complete {
                "-".into()
            } else {
                "?".into()
            }
        } else {
            scope_value(partial(value, unknown, applicable))
        }
    };
    let active = if totals.activity_unknown > 0 || !totals.inventory_complete {
        if totals.active_containers == 0 {
            "?".into()
        } else {
            format!(">={}", totals.active_containers)
        }
    } else {
        totals.active_containers.to_string()
    };
    let age = if applicable == 0 && totals.inventory_complete {
        "-".into()
    } else {
        uptime(totals.oldest_uptime_seconds)
    };
    let age = if (totals.uptime_unknown > 0 || !totals.inventory_complete)
        && totals.oldest_uptime_seconds.is_some()
    {
        format!(">={age}")
    } else {
        age
    };
    format!(
        "{reference} · {} · {} · oldest {age} · {active} active / {} containers{collapsed}",
        usage(percent(Some(totals.known_cpu_percent)), totals.cpu_unknown),
        usage(
            memory(Some(totals.known_memory_bytes)),
            totals.memory_unknown
        ),
        scope_value(totals.containers.to_string())
    )
}

pub(super) fn row(node: &Node, container: &Container) -> String {
    let ports = container.ports.as_ref().map_or("?".into(), |ports| {
        let published: BTreeSet<_> = ports
            .iter()
            .filter_map(|port| {
                port.host_port
                    .map(|host| format!("{}:{host}->{}", port.protocol, port.container_port))
            })
            .collect();
        if published.is_empty() {
            "-".into()
        } else {
            published.into_iter().collect::<Vec<_>>().join(",")
        }
    });
    let metric = |value: String| {
        if container.is_inactive() {
            "-".into()
        } else {
            value
        }
    };
    format!(
        "{} [container] · {} · {} · {} · {} · {} · {ports}",
        name_markdown(&node.name),
        node.reference,
        metric(percent(container.cpu_percent)),
        metric(memory(container.memory_bytes)),
        metric(uptime(container.uptime_seconds)),
        container.state.as_deref().map_or("?".into(), escaped)
    )
}

pub(super) fn notes(node: &Node, details: bool) -> Vec<String> {
    let mut notes = Vec::new();
    if let Some(runtime) = &node.runtime {
        notes.extend(
            runtime
                .unavailable
                .iter()
                .map(|issue| format!("! {}", escaped(issue))),
        );
    }
    if let Some(container) = &node.container {
        if details && let Some(project) = &container.project {
            let source = match project.source {
                ProjectSource::Compose => "compose",
                ProjectSource::BindWorkdir => "~ bind cwd + project marker",
                ProjectSource::BindProject => "~ single bind project + marker",
            };
            let path = project
                .path
                .as_ref()
                .map(|path| format!(" {}", code(&path.to_string_lossy())))
                .unwrap_or_default();
            notes.push(format!("{source} {}{path}", escaped(&project.name)));
        }
        if details {
            notes.extend(
                container
                    .unavailable
                    .iter()
                    .map(|issue| format!("! {}", escaped(issue))),
            );
        }
        if !details && container.project.is_none() && container.candidates.len() > 1 {
            notes.push(format!("{} project candidates", container.candidates.len()));
        }
        if details {
            notes.push(format!(
                "{} {}",
                escaped(&container.runtime),
                escaped(&container.id)
            ));
            if let Some(start) = &container.started_at {
                notes.push(format!("started {}", escaped(start)));
            }
            if let Some(service) = &container.service {
                notes.push(format!("service {}", escaped(service)));
            }
            for mount in &container.mounts {
                notes.push(format!(
                    "{} {} -> {}",
                    escaped(&mount.kind),
                    code(&mount.source.to_string_lossy()),
                    code(&mount.destination.to_string_lossy())
                ));
            }
            for path in &container.candidates {
                notes.push(format!("candidate {}", code(&path.to_string_lossy())));
            }
            if let Some(ports) = &container.ports {
                for port in ports {
                    if let (Some(ip), Some(host)) = (&port.host_ip, port.host_port) {
                        notes.push(format!(
                            "published {} [{}]:{host} -> {}",
                            escaped(&port.protocol),
                            escaped(ip),
                            port.container_port
                        ));
                    }
                }
            }
        }
    }
    notes
}
