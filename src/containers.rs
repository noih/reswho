use crate::model::{Attribution, AttributionSource};
#[cfg(target_os = "macos")]
pub(crate) mod apple;
mod attribution;
pub(crate) mod command;
pub(crate) mod docker;
mod http;
#[cfg(test)]
mod measure;
mod model;
mod podman;
mod stats;

use crate::{
    extensions::host::stable_ref,
    model::{Grouping, Node},
};
use anyhow::{Result, bail};
pub use model::{
    Container, ContainerTotals, DiskUsage, DiskUsageEntry, Mount, Port, Project, ProjectSource,
    Runtime,
};
use std::{
    collections::{BTreeMap, HashSet},
    time::Duration,
};

pub fn collect() -> Result<Node> {
    collect_with_cancel(
        &crate::snapshot::Cancellation::default(),
        Duration::from_millis(250),
    )
}

pub(crate) fn collect_with_cancel(
    cancel: &crate::snapshot::Cancellation,
    sample_interval: Duration,
) -> Result<Node> {
    let settings = crate::extensions::settings_registry()?.resolve(
        &crate::settings::Config::default(),
        &BTreeMap::from([(
            "sampling.interval_ms".into(),
            serde_json::Value::from(sample_interval.as_millis() as u64),
        )]),
    )?;
    collect_with_context(&crate::extensions::Context::new(&settings, cancel))
}

pub(crate) fn collect_with_context(context: &crate::extensions::Context<'_>) -> Result<Node> {
    let providers: Vec<&dyn crate::extensions::Extension> = crate::extensions::BUILTINS
        .iter()
        .filter(|extension| {
            use crate::extensions::Extension;
            extension.descriptor().enabled
                && extension
                    .descriptor()
                    .capabilities
                    .contains(&crate::extensions::Capability::Collect)
                && (!matches!(extension, crate::extensions::Builtin::Apple)
                    || command::find_program("container").is_some())
                && (!matches!(extension, crate::extensions::Builtin::Podman) || podman_available())
        })
        .map(|extension| extension as &dyn crate::extensions::Extension)
        .collect();
    crate::extensions::collect(&providers, context)
}

pub(crate) fn apple_available() -> bool {
    #[cfg(target_os = "macos")]
    {
        command::find_program("container").is_some()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

pub(crate) fn collect_docker(
    context: &crate::extensions::Context<'_>,
) -> Result<(Runtime, Vec<Container>)> {
    let mut runner = command::RuntimeCommand {
        cancel: context.cancel.clone(),
        program: "docker".into(),
        deadline: context.deadline,
        sample_interval: context.settings.interval("sampling.interval_ms"),
        api_version: std::env::var("DOCKER_API_VERSION").ok(),
    };
    Ok(docker::collect(
        &mut runner,
        std::env::var("DOCKER_CONTEXT").ok(),
        std::env::var("DOCKER_HOST").ok(),
        time::OffsetDateTime::now_utc(),
        context.detail_refs,
    ))
}

pub(crate) fn podman_available() -> bool {
    command::find_program("podman").is_some()
}

pub(crate) fn collect_podman(
    context: &crate::extensions::Context<'_>,
) -> Result<(Runtime, Vec<Container>)> {
    Ok(podman::native(
        context.cancel,
        context.deadline,
        context.settings.interval("sampling.interval_ms"),
    ))
}

pub(crate) fn collect_apple(
    context: &crate::extensions::Context<'_>,
) -> Result<(Runtime, Vec<Container>)> {
    #[cfg(target_os = "macos")]
    {
        let program = command::find_program("container")
            .ok_or_else(|| anyhow::anyhow!("Apple Container CLI unavailable"))?;
        Ok(apple::native(
            program,
            context.cancel,
            context.deadline,
            context.settings.interval("sampling.interval_ms"),
        ))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = context;
        bail!("Apple Container requires macOS");
    }
}

pub(crate) fn retain_host_metadata(
    enabled: bool,
    runtime: &mut Runtime,
    containers: &mut [Container],
) {
    if !enabled {
        runtime.host_scope = None;
        runtime.metrics_scope = None;
        runtime.configuration = None;
        for container in containers {
            container.host_scope = None;
            container.configuration = None;
            container.metrics_scope = None;
        }
    }
}

pub(crate) fn build_all(sources: Vec<(Runtime, Vec<Container>)>) -> Result<Node> {
    let mut root = Node::group(
        "group:containers".into(),
        "Containers".into(),
        vec!["Container scope; excluded from host process totals".into()],
        Grouping::Category,
    );
    let mut totals = ContainerTotals {
        inventory_complete: true,
        ..ContainerTotals::default()
    };
    for (runtime, containers) in sources {
        totals.inventory_complete &= runtime.inventory_complete;
        for container in &containers {
            totals.add(container);
        }
        root.children.extend(build(runtime, containers)?.children);
    }
    root.container_totals = Some(totals);
    crate::tree::validate_references(&root)?;
    Ok(root)
}

pub(crate) fn validate_containers(containers: &[Container]) -> Result<()> {
    let mut ids = HashSet::new();
    for container in containers {
        if !ids.insert(&container.id) {
            bail!("duplicate container identity");
        }
    }
    Ok(())
}

fn build(runtime: Runtime, containers: Vec<Container>) -> Result<Node> {
    validate_containers(&containers)?;
    let runtime_key = runtime.key();
    let label = runtime
        .context
        .as_deref()
        .unwrap_or(if runtime.endpoint_ref.is_some() {
            "DOCKER_HOST"
        } else {
            "unresolved"
        });
    let mut runtime_node = Node::group(
        runtime.reference(),
        format!("{} ({label})", runtime.name),
        Vec::new(),
        Grouping::Runtime,
    );
    let mut totals = ContainerTotals {
        inventory_complete: runtime.inventory_complete,
        ..ContainerTotals::default()
    };
    let mut projects: BTreeMap<String, Node> = BTreeMap::new();
    for container in containers {
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
    runtime_node.runtime = Some(Box::new(runtime));
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
