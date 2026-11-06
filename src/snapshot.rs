use crate::{collect, containers, model::Snapshot, tree};
use anyhow::{Context, Result};
use std::{
    sync::Arc,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

mod cancellation;
pub use cancellation::Cancellation;

#[derive(Clone)]
pub struct Options {
    pub settings: Arc<crate::settings::Resolved>,
    pub containers: bool,
    pub command_pids: Vec<u32>,
    pub detail_refs: Vec<String>,
}

pub(crate) struct DetailSnapshot {
    pub captured_unix_ms: u64,
    pub node: Option<crate::model::Node>,
}

pub(crate) fn capture_detail(
    options: &Options,
    cancel: &Cancellation,
    target: &crate::model::Node,
    report: Arc<std::sync::Mutex<Vec<crate::extensions::report::Record>>>,
) -> Result<DetailSnapshot> {
    use crate::extensions::Extension;
    cancel.check()?;
    let mut context = crate::extensions::Context::new(&options.settings, cancel);
    *report.lock().expect("report lock") = Vec::new();
    context.report = report;
    let refs = [target.reference.clone()];
    context.detail_refs = &refs;
    let mut node = if let Some(expected) = &target.process {
        if !collect::verify_identity(expected) {
            None
        } else {
            let mut processes = collect::detail_processes(&context, expected)?;
            let valid = processes.first().is_some_and(|fresh| {
                fresh.started_unix_us == expected.started_unix_us
                    && fresh.uid == expected.uid
                    && fresh.executable == expected.executable
            });
            if !valid {
                None
            } else {
                let mut root = tree::build_with_providers(
                    processes.clone(),
                    &context,
                    &crate::extensions::default_classifiers(),
                )?;
                let providers: Vec<&dyn Extension> = crate::extensions::BUILTINS
                    .iter()
                    .filter(|provider| {
                        provider
                            .descriptor()
                            .capabilities
                            .contains(&crate::extensions::Capability::Details)
                    })
                    .map(|provider| provider as &dyn Extension)
                    .collect();
                selected_details(
                    &mut root,
                    &mut processes,
                    &[expected.pid],
                    &providers,
                    &context,
                    collect::verify_identity,
                );
                if collect::verify_identity(expected) {
                    tree::select_detail(&root, &target.reference).ok()
                } else {
                    None
                }
            }
        }
    } else {
        let source = resource_source(target).context("Resource has no detail collector")?;
        anyhow::ensure!(
            (options.containers || target.resource.is_some())
                && !options.settings.is_disabled(source),
            "Resource source disabled"
        );
        let providers: Vec<&dyn Extension> = crate::extensions::BUILTINS
            .iter()
            .map(|provider| provider as &dyn Extension)
            .collect();
        resource_detail(target, &providers, &context)?
    };
    context.check()?;
    if let Some(node) = &mut node {
        retain_overview_context(node, target);
        node.children.clear();
    }
    Ok(DetailSnapshot {
        captured_unix_ms: SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64,
        node,
    })
}

fn retain_overview_context(node: &mut crate::model::Node, target: &crate::model::Node) {
    node.host_resources = target.host_resources.clone();
    let mut retained = !node.host_resources.is_empty();
    if let Some(fresh) = node.process.as_mut().and_then(|p| p.host_resource.as_mut())
        && let Some(old) = target
            .process
            .as_ref()
            .and_then(|p| p.host_resource.as_ref())
        && fresh.kind == old.kind
        && fresh.scope == old.scope
        && fresh.link_key.is_some()
        && fresh.link_key == old.link_key
        && fresh.configuration.is_none()
    {
        fresh.configuration = old.configuration.clone();
        retained |= fresh.configuration.is_some();
    }
    if retained {
        node.diagnostics.push(crate::model::Diagnostic::new("not_refreshed", "detail.overview_context",
            "Linked host usage/configuration is from the overview snapshot; not refreshed by this query"));
    }
}

fn resource_source(target: &crate::model::Node) -> Option<&str> {
    target
        .runtime
        .as_ref()
        .map(|runtime| runtime.name.as_str())
        .or_else(|| {
            target
                .container
                .as_ref()
                .map(|container| container.runtime.as_str())
        })
        .or_else(|| {
            target
                .resource
                .as_ref()
                .map(|resource| resource.provider.as_str())
        })
}

fn resource_detail(
    target: &crate::model::Node,
    providers: &[&dyn crate::extensions::Extension],
    context: &crate::extensions::Context<'_>,
) -> Result<Option<crate::model::Node>> {
    let source = resource_source(target).context("Resource identity unavailable")?;
    let selected: Vec<_> = providers
        .iter()
        .copied()
        .filter(|provider| provider.descriptor().id == source)
        .collect();
    anyhow::ensure!(!selected.is_empty(), "Resource source unavailable");
    let (containers, resources) =
        crate::extensions::collect_scoped(&selected, context, target.resource.is_none())?;
    let root = if target.resource.is_some() {
        resources
    } else {
        containers
    };
    Ok(tree::select_detail(&root, &target.reference).ok())
}

pub fn capture(options: &Options, cancel: &Cancellation) -> Result<Snapshot> {
    capture_reported(options, cancel, Arc::default())
}

pub fn capture_reported(
    options: &Options,
    cancel: &Cancellation,
    report: Arc<std::sync::Mutex<Vec<crate::extensions::report::Record>>>,
) -> Result<Snapshot> {
    let start = Instant::now();
    *report.lock().expect("report lock") =
        crate::extensions::inventory::initial_report(&options.settings, options.containers);
    cancel.check()?;
    let mut context = crate::extensions::Context::new(&options.settings, cancel);
    context.report = Arc::clone(&report);
    context.detail_refs = &options.detail_refs;
    let mut collection = collect::collect_with_context(&context, &options.command_pids)?;
    cancel.check()?;
    let mut context = crate::extensions::Context::new(&options.settings, cancel);
    context.report = Arc::clone(&report);
    context.detail_refs = &options.detail_refs;
    let classifiers = [&crate::core::Core::AppBundle as &dyn crate::extensions::Extension];
    let mut root =
        tree::build_with_providers(collection.processes.clone(), &context, &classifiers)?;
    let providers: Vec<&dyn crate::extensions::Extension> = crate::extensions::BUILTINS
        .iter()
        .filter(|provider| {
            use crate::extensions::Extension;
            provider
                .descriptor()
                .capabilities
                .contains(&crate::extensions::Capability::Details)
        })
        .map(|provider| provider as &dyn crate::extensions::Extension)
        .collect();
    selected_details(
        &mut root,
        &mut collection.processes,
        &options.command_pids,
        &providers,
        &context,
        crate::collect::verify_identity,
    );
    {
        let providers: Vec<&dyn crate::extensions::Extension> = crate::extensions::BUILTINS
            .iter()
            .filter(|provider| {
                use crate::extensions::{Builtin, Extension};
                options.containers
                    && provider
                        .descriptor()
                        .capabilities
                        .contains(&crate::extensions::Capability::Collect)
                    && (!matches!(provider, Builtin::Apple) || containers::apple_available())
                    && (!matches!(provider, Builtin::Podman) || containers::podman_available())
            })
            .map(|provider| provider as &dyn crate::extensions::Extension)
            .collect();
        match crate::extensions::collect_scoped(&providers, &context, options.containers) {
            Ok((containers, resources)) => {
                for (category, include) in [(containers, options.containers), (resources, true)] {
                    if include
                        && !category.children.is_empty()
                        && tree::compose_category(&mut root, category).is_err()
                    {
                        collection.warnings.push(crate::model::Diagnostic::new(
                            "collection_failed",
                            "categories",
                            "Category contribution rejected: incompatible or colliding content",
                        ));
                    }
                }
            }
            Err(error) => collection.warnings.push(crate::model::Diagnostic::new(
                "collection_failed",
                "containers",
                format!("Containers: {error}"),
            )),
        }
    }
    cancel.check()?;
    tree::validate_references(&root)?;
    let mut snapshot = Snapshot {
        schema: crate::model::JSON_SCHEMA_VERSION,
        extensions: report.lock().expect("report lock").clone(),
        captured_unix_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock before Unix epoch")?
            .as_millis() as u64,
        sample_ms: collection.sample_ms,
        collection_ms: start.elapsed().as_millis() as u64,
        system: collection.system,
        scope: "group:all".into(),
        warnings: collection.warnings,
        root,
    };
    tree::link_host_resources(&mut snapshot.root);
    snapshot.project(&options.command_pids);
    Ok(snapshot)
}

fn selected_details(
    root: &mut crate::model::Node,
    processes: &mut [crate::model::Process],
    pids: &[u32],
    providers: &[&dyn crate::extensions::Extension],
    context: &crate::extensions::Context<'_>,
    verify: impl Fn(&crate::model::Process) -> bool,
) {
    if pids.is_empty() {
        return;
    }
    let pids: std::collections::HashSet<_> = pids.iter().copied().collect();
    let mut updates = std::collections::HashMap::new();
    for process in processes
        .iter_mut()
        .filter(|process| pids.contains(&process.pid) && process.started_unix_us.is_some())
    {
        let diagnostics = crate::extensions::details::attach_details(providers, context, process);
        if !providers.is_empty() {
            validate_details_identity(context, process, &verify);
        }
        updates.insert(process.pid, (&*process, diagnostics));
    }
    attach_details(root, &updates);
}

fn attach_details(
    node: &mut crate::model::Node,
    updates: &std::collections::HashMap<
        u32,
        (&crate::model::Process, Vec<crate::model::Diagnostic>),
    >,
) {
    if let Some(old) = node.process.as_mut()
        && let Some((process, diagnostics)) = updates.get(&old.pid)
    {
        let mut updated = (*process).clone();
        updated.tag_report = old.tag_report.take();
        updated.roles = std::mem::take(&mut old.roles);
        updated.name = std::mem::take(&mut old.name);
        updated.original_name = old.original_name.take();
        updated.name_source = old.name_source.take();
        updated.version = old.version.take();
        updated.host_resource = old.host_resource.take();
        updated.supplementary_labels = std::mem::take(&mut old.supplementary_labels);
        node.process = Some(Box::new(updated));
        node.diagnostics.extend_from_slice(diagnostics);
    }
    for child in &mut node.children {
        attach_details(child, updates);
    }
}

fn validate_details_identity(
    context: &crate::extensions::Context<'_>,
    process: &mut crate::model::Process,
    verify: impl FnOnce(&crate::model::Process) -> bool,
) {
    let validation_started = Instant::now();
    if !verify(process) {
        context.reject_details_identity(process, validation_started.elapsed());
        process.extension_details.clear();
        process.unavailable.push(crate::model::Diagnostic::new(
            "identity_changed",
            "extensions.details",
            "Process identity changed during extension details",
        ));
    }
}

#[cfg(test)]
mod tests;
