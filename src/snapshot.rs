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
    let mut collection = collect::collect_with_context(&context, &options.command_pids)?;
    cancel.check()?;
    let mut context = crate::extensions::Context::new(&options.settings, cancel);
    context.report = Arc::clone(&report);
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
    for process in processes
        .iter_mut()
        .filter(|process| pids.contains(&process.pid) && process.started_unix_us.is_some())
    {
        let diagnostics = crate::extensions::details::attach_details(providers, context, process);
        if !providers.is_empty() {
            validate_details_identity(context, process, &verify);
        }
        attach_details(root, process, &diagnostics);
    }
}

fn attach_details(
    node: &mut crate::model::Node,
    process: &crate::model::Process,
    diagnostics: &[crate::model::Diagnostic],
) {
    if let Some(old) = node.process.as_mut().filter(|old| old.pid == process.pid) {
        let mut updated = process.clone();
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
        attach_details(child, process, diagnostics);
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
