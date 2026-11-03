use crate::{collect, containers, model::Snapshot, tree};
use anyhow::{Context, Result, bail};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);

impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    pub fn check(&self) -> Result<()> {
        if self.0.load(Ordering::Relaxed) {
            bail!("collection canceled");
        }
        Ok(())
    }
    pub fn wait(&self, duration: Duration) -> Result<()> {
        let start = Instant::now();
        loop {
            self.check()?;
            let remaining = duration.saturating_sub(start.elapsed());
            if remaining.is_zero() {
                return Ok(());
            }
            std::thread::sleep(remaining.min(Duration::from_millis(20)));
        }
    }
}

#[derive(Clone)]
pub struct Options {
    pub settings: Arc<crate::settings::Resolved>,
    pub extensions: Arc<crate::extensions::state::State>,
    pub containers: bool,
    pub command_pid: Option<u32>,
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
    *report.lock().expect("report lock") = options
        .extensions
        .initial_report(&options.settings, options.containers);
    cancel.check()?;
    let mut context = crate::extensions::Context::new(&options.settings, cancel);
    context.report = Arc::clone(&report);
    let mut collection = collect::collect_with_context(&context, options.command_pid)?;
    cancel.check()?;
    let mut context = crate::extensions::Context::new(&options.settings, cancel);
    context.report = Arc::clone(&report);
    let external = crate::extensions::external::installed(&options.extensions, &options.settings);
    let classifiers: Vec<&dyn crate::extensions::Extension> = std::iter::once(
        &crate::extensions::Builtin::AppBundle as &dyn crate::extensions::Extension,
    )
    .chain(
        external
            .iter()
            .filter(|provider| {
                provider
                    .manifest
                    .known_capabilities()
                    .contains(&crate::extensions::Capability::Classify)
            })
            .map(|provider| provider as &dyn crate::extensions::Extension),
    )
    .collect();
    let mut root =
        tree::build_with_providers(collection.processes.clone(), &context, &classifiers)?;
    if let Some(pid) = options.command_pid
        && let Some(process) = collection
            .processes
            .iter_mut()
            .find(|process| process.pid == pid && process.started_unix_us.is_some())
    {
        let providers: Vec<&dyn crate::extensions::Extension> = external
            .iter()
            .filter(|provider| {
                provider
                    .manifest
                    .known_capabilities()
                    .contains(&crate::extensions::Capability::Details)
            })
            .map(|provider| provider as &dyn crate::extensions::Extension)
            .collect();
        let diagnostics =
            crate::extensions::external::attach_details(&providers, &context, process);
        if !providers.is_empty() {
            validate_external_details(&context, process, crate::collect::verify_identity);
        }
        fn attach(node: &mut crate::model::Node, process: &crate::model::Process) {
            if node
                .process
                .as_ref()
                .is_some_and(|old| old.pid == process.pid)
            {
                let mut updated = process.clone();
                updated.tag_report = node.process.as_ref().and_then(|old| old.tag_report.clone());
                updated.roles = node
                    .process
                    .as_ref()
                    .expect("selected process")
                    .roles
                    .clone();
                node.process = Some(updated);
            }
            for child in &mut node.children {
                attach(child, process);
            }
        }
        attach(&mut root, process);
        root.diagnostics.extend(diagnostics);
    }
    {
        let mut providers: Vec<&dyn crate::extensions::Extension> = crate::extensions::BUILTINS
            .iter()
            .filter(|provider| {
                use crate::extensions::{Builtin, Extension};
                options.containers
                    && provider
                        .descriptor()
                        .capabilities
                        .contains(&crate::extensions::Capability::Collect)
                    && (!matches!(provider, Builtin::Apple) || containers::apple_available())
            })
            .map(|provider| provider as &dyn crate::extensions::Extension)
            .collect();
        providers.extend(
            external
                .iter()
                .filter(|provider| {
                    provider
                        .manifest
                        .known_capabilities()
                        .contains(&crate::extensions::Capability::Collect)
                        && (options.containers
                            || provider
                                .manifest
                                .collect_scopes
                                .iter()
                                .any(|scope| scope == "resources"))
                })
                .map(|provider| provider as &dyn crate::extensions::Extension),
        );
        match crate::extensions::collect_scoped(&providers, &context, options.containers) {
            Ok((containers, resources)) => {
                if options.containers {
                    root.children.push(containers);
                }
                if !resources.children.is_empty() {
                    root.children.push(resources);
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
    snapshot.project(options.command_pid);
    Ok(snapshot)
}

fn validate_external_details(
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
            "Process identity changed during external details",
        ));
    }
}

#[cfg(test)]
mod tests;
