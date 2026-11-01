use crate::model::{Attribution, AttributionSource};
mod attribution;
mod command;
mod docker;
mod model;

use crate::{
    model::{Grouping, Node},
    tree::stable_ref,
};
use anyhow::{Result, bail};
pub use model::{Container, ContainerTotals, Mount, Port, Project, ProjectSource, Runtime};
use std::{
    collections::{BTreeMap, HashSet},
    time::{Duration, Instant},
};

pub fn collect() -> Result<Node> {
    collect_with_cancel(&crate::snapshot::Cancellation::default())
}

pub(crate) fn collect_with_cancel(cancel: &crate::snapshot::Cancellation) -> Result<Node> {
    let mut runner = command::DockerCommand {
        cancel: cancel.clone(),
        program: "docker".into(),
        deadline: Instant::now() + Duration::from_secs(10),
    };
    let (runtime, containers) = docker::collect(
        &mut runner,
        std::env::var("DOCKER_CONTEXT").ok(),
        std::env::var("DOCKER_HOST").ok(),
        time::OffsetDateTime::now_utc(),
    );
    build(runtime, containers)
}

fn build(runtime: Runtime, containers: Vec<Container>) -> Result<Node> {
    let endpoint = runtime.endpoint_ref.as_deref().unwrap_or("unresolved");
    let runtime_key = format!(
        "{}\0{endpoint}\0{}",
        runtime.name,
        runtime.daemon_id.as_deref().unwrap_or("unknown")
    );
    let label = runtime
        .context
        .as_deref()
        .unwrap_or(if runtime.endpoint_ref.is_some() {
            "DOCKER_HOST"
        } else {
            "unresolved"
        });
    let mut runtime_node = Node::group(
        stable_ref("runtime", &runtime_key),
        format!("{} ({label})", runtime.name),
        Vec::new(),
        Grouping::Runtime,
    );
    let mut totals = ContainerTotals {
        inventory_complete: runtime.inventory_complete,
        ..ContainerTotals::default()
    };
    let mut projects: BTreeMap<String, Node> = BTreeMap::new();
    let mut ids = HashSet::new();
    for container in containers {
        if !ids.insert(container.id.clone()) {
            bail!("duplicate container identity");
        }
        totals.add(&container);
        let project_key = container
            .project
            .as_ref()
            .map(|project| {
                serde_json::to_string(&(
                    matches!(project.source, ProjectSource::Compose),
                    &project.name,
                    &project.path,
                ))
            })
            .transpose()?;
        let mut node = Node::group(
            stable_ref("container", &format!("{runtime_key}\0{}", container.id)),
            container.name.clone(),
            Vec::new(),
            Grouping::Container,
        );
        node.kind = "container";
        if let Some(project) = &container.project {
            let source = match project.source {
                ProjectSource::Compose => AttributionSource::Compose,
                ProjectSource::BindWorkdir => AttributionSource::BindWorkdir,
                ProjectSource::BindProject => AttributionSource::BindProject,
            };
            let mut attribution = Attribution::new(source, project.path.clone());
            attribution.inferred = !matches!(project.source, ProjectSource::Compose);
            node.attribution = Some(attribution);
        }
        if let (Some(project), Some(project_key)) = (&container.project, project_key) {
            let project_node = projects.entry(project_key.clone()).or_insert_with(|| {
                let mut group = Node::group(
                    stable_ref(
                        "container-project",
                        &format!("{runtime_key}\0{project_key}"),
                    ),
                    project.name.clone(),
                    Vec::new(),
                    Grouping::Project,
                );
                match project.source {
                    ProjectSource::Compose => {
                        group.evidence.push(project.path.as_ref().map_or_else(
                            || "compose".into(),
                            |path| format!("compose {}", path.display()),
                        ))
                    }
                    _ => {
                        group
                            .evidence
                            .push("Container project inferred: bind path + project marker".into());
                        if let Some(path) = &project.path {
                            group
                                .evidence
                                .push(format!("container project path {}", path.display()));
                        }
                    }
                }
                group.attribution = node.attribution.clone();
                group.container_totals = Some(ContainerTotals {
                    inventory_complete: runtime.inventory_complete,
                    ..ContainerTotals::default()
                });
                group
            });
            if let Some(totals) = project_node.container_totals.as_mut() {
                totals.add(&container);
            }
            node.container = Some(container);
            project_node.children.push(node);
        } else {
            node.container = Some(container);
            runtime_node.children.push(node);
        }
    }
    runtime_node.children.extend(projects.into_values());
    runtime_node
        .children
        .sort_by(|a, b| a.name.cmp(&b.name).then(a.reference.cmp(&b.reference)));
    runtime_node.container_totals = Some(totals.clone());
    runtime_node.runtime = Some(runtime);
    let mut root = Node::group(
        "group:containers".into(),
        "Containers".into(),
        vec!["Container scope; excluded from host process totals".into()],
        Grouping::Category,
    );
    root.container_totals = Some(totals);
    root.children.push(runtime_node);
    let mut seen = HashSet::new();
    let mut pending = vec![&root];
    while let Some(node) = pending.pop() {
        if !seen.insert(&node.reference) {
            bail!("container group identity collision");
        }
        pending.extend(&node.children);
    }
    Ok(root)
}

#[cfg(test)]
mod tests;
