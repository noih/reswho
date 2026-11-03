mod discovery;
pub mod external;
mod native;
pub mod protocol;
pub mod report;
mod resources;
pub mod state;
use crate::{
    containers::{self, Container, Runtime},
    model::{Diagnostic, Process, ProcessCommand},
    settings::{Config, Registry, Resolved, Use},
    snapshot::Cancellation,
};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    cell::Cell,
    collections::{BTreeMap, HashSet},
    panic::{self, AssertUnwindSafe},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Once},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Collect,
    Classify,
    Details,
}

#[derive(Serialize)]
pub struct Descriptor {
    pub id: String,
    pub version: String,
    pub protocol_version: u32,
    pub source: &'static str,
    pub platforms: Vec<String>,
    pub capabilities: Vec<Capability>,
    pub enabled: bool,
    pub availability: &'static str,
}

pub struct Context<'a> {
    pub settings: &'a Resolved,
    pub cancel: &'a Cancellation,
    pub deadline: Instant,
    failed: Mutex<HashSet<String>>,
    pub report: Arc<Mutex<Vec<report::Record>>>,
}

impl<'a> Context<'a> {
    pub fn new(settings: &'a Resolved, cancel: &'a Cancellation) -> Self {
        Self {
            settings,
            cancel,
            deadline: Instant::now() + Duration::from_secs(10),
            failed: Mutex::new(HashSet::new()),
            report: Arc::default(),
        }
    }
    fn finish_phase<T>(
        &self,
        provider: Option<&str>,
        phase: &str,
        validation: Duration,
        result: Result<T>,
    ) -> Result<T> {
        if let Some(record) = self
            .report
            .lock()
            .expect("report lock")
            .iter_mut()
            .rev()
            .find(|record| Some(record.provider.as_str()) == provider && record.phase == phase)
        {
            if let Some(duration) = &mut record.duration_ms {
                *duration += validation.as_secs_f64() * 1000.0;
            }
            if result.is_err() && record.result == "success" {
                record.result = "failed".into();
                record.reason = Some("invalid_data".into());
            }
        }
        self.finish(provider, result)
    }
    pub(crate) fn reject_details_identity(&self, process: &Process, validation: Duration) {
        for details in &process.extension_details {
            let rejected = {
                let mut records = self.report.lock().expect("report lock");
                if let Some(record) = records.iter_mut().rev().find(|record| {
                    record.provider == details.provider
                        && record.version.as_deref() == Some(details.version.as_str())
                        && record.phase == "details"
                }) && record.result == "success"
                {
                    record.result = "failed".into();
                    record.reason = Some("identity_changed".into());
                    if let Some(duration) = &mut record.duration_ms {
                        *duration += validation.as_secs_f64() * 1000.0;
                    }
                    true
                } else {
                    false
                }
            };
            if rejected {
                let _ = self.finish::<()>(
                    Some(&details.provider),
                    Err(anyhow::anyhow!("details identity changed")),
                );
            }
        }
    }
    fn finish<T>(&self, provider: Option<&str>, result: Result<T>) -> Result<T> {
        if result.is_err()
            && let Some(id) = provider
        {
            self.failed
                .lock()
                .expect("round failure lock")
                .insert(id.into());
        }
        result
    }
    pub fn check(&self) -> Result<()> {
        self.cancel.check()?;
        if Instant::now() >= self.deadline {
            bail!("extension deadline exceeded");
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub started_unix_us: u64,
}

impl ProcessIdentity {
    pub fn observed(process: &Process) -> Option<Self> {
        Some(Self {
            pid: process.pid,
            started_unix_us: process.started_unix_us?,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ClassificationEvidence {
    AppBundle {
        path: PathBuf,
    },
    Role {
        key: String,
        label: String,
        path: PathBuf,
        basis: String,
    },
    Custom {
        key: String,
        label: String,
        path: PathBuf,
        basis: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClassificationProposal {
    pub identity: ProcessIdentity,
    pub evidence: ClassificationEvidence,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DetailProposal {
    pub identity: ProcessIdentity,
    pub command: ProcessCommand,
}

#[derive(Clone, Copy)]
pub enum Request<'a> {
    Collect,
    CollectResources,
    Classify(&'a [Process]),
    Details {
        process: &'a Process,
        pointer_width: usize,
    },
}

impl Request<'_> {
    fn capability(&self) -> Capability {
        match self {
            Self::Collect | Self::CollectResources => Capability::Collect,
            Self::Classify(_) => Capability::Classify,
            Self::Details { .. } => Capability::Details,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "capability", rename_all = "snake_case")]
pub enum Contribution {
    Collect {
        runtime: Runtime,
        containers: Vec<Container>,
    },
    Collected {
        runtime: Option<Runtime>,
        containers: Vec<Container>,
        resources: Vec<crate::model::Resource>,
    },
    Classify {
        proposals: Vec<ClassificationProposal>,
    },
    Details {
        proposal: DetailProposal,
    },
    ParsedDetails {
        identity: ProcessIdentity,
        fields: Vec<crate::model::DetailField>,
    },
}

impl Contribution {
    fn capability(&self) -> Capability {
        match self {
            Self::Collect { .. } | Self::Collected { .. } => Capability::Collect,
            Self::Classify { .. } => Capability::Classify,
            Self::Details { .. } | Self::ParsedDetails { .. } => Capability::Details,
        }
    }
}

#[derive(Serialize)]
pub struct Response {
    pub provider: String,
    pub version: String,
    pub contribution: Contribution,
}

pub trait Extension: Sync {
    fn collects_containers(&self) -> bool {
        true
    }
    fn descriptor(&self) -> Descriptor;
    fn execute(&self, context: &Context<'_>, request: Request<'_>) -> Result<Contribution>;
}

thread_local! {
    static IN_EXTENSION: Cell<bool> = const { Cell::new(false) };
}

fn boundary<T>(operation: impl FnOnce() -> Result<T>) -> Result<T> {
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            if !IN_EXTENSION.try_with(Cell::get).unwrap_or(false) {
                previous(info);
            }
        }));
    });
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            IN_EXTENSION.with(|active| active.set(self.0));
        }
    }
    let _restore = Restore(IN_EXTENSION.with(|active| active.replace(true)));
    match panic::catch_unwind(AssertUnwindSafe(operation)) {
        Ok(result) => result,
        Err(_) => Err(anyhow::anyhow!("extension panicked")),
    }
}

pub fn dispatch(
    extension: &dyn Extension,
    context: &Context<'_>,
    request: Request<'_>,
) -> Result<Response> {
    dispatch_labeled(extension, context, request).1
}

fn dispatch_labeled(
    extension: &dyn Extension,
    context: &Context<'_>,
    request: Request<'_>,
) -> (Option<String>, Result<Response>) {
    let mut provider = None;
    let mut version = None;
    let mut invoked = false;
    let started = Instant::now();
    let phase = format!("{:?}", request.capability()).to_lowercase();
    let result = boundary(|| {
        let descriptor = extension.descriptor();
        provider = Some(descriptor.id.clone());
        version = Some(descriptor.version.clone());
        context.check()?;
        if context
            .failed
            .lock()
            .expect("round failure lock")
            .contains(&descriptor.id)
        {
            bail!("extension failed earlier in this round");
        }
        let capability = request.capability();
        if !descriptor.enabled
            || context
                .settings
                .report()
                .disabled_extensions
                .contains_key(&descriptor.id)
            || !descriptor.capabilities.contains(&capability)
        {
            bail!("extension capability is disabled or unsupported");
        }
        invoked = true;
        let contribution = extension
            .execute(context, request)
            .map_err(|_| anyhow::anyhow!("extension failed"))?;
        context.check()?;
        if contribution.capability() != capability {
            bail!("extension returned an unexpected capability");
        }
        Ok(Response {
            provider: descriptor.id,
            version: descriptor.version,
            contribution,
        })
    });
    let result = context.finish(provider.as_deref(), result);
    let reason = if context.cancel.check().is_err() {
        "canceled"
    } else if Instant::now() >= context.deadline {
        "deadline"
    } else if provider.is_none() {
        "metadata_failed"
    } else if invoked {
        "provider_failed"
    } else {
        "not_enabled_or_prior_failure"
    };
    let record = report::Record::new(
        provider.as_deref().unwrap_or("unidentified"),
        version.as_deref(),
        &phase,
        if result.is_ok() {
            "success"
        } else if invoked || provider.is_none() {
            "failed"
        } else {
            "skipped"
        },
        result.as_ref().err().map(|_| reason),
        (invoked || provider.is_none()).then_some(started),
    );
    let mut records = context.report.lock().expect("report lock");
    if let Some(index) = records.iter().position(|old| {
        old.provider == record.provider && old.phase == phase && old.started.is_none()
    }) {
        records[index] = record;
    } else if records.len() < 2048 {
        records.push(record);
    }
    (provider, result)
}

pub enum Builtin {
    Docker,
    Apple,
    AppBundle,
    Command,
}

pub const BUILTINS: [Builtin; 4] = [
    Builtin::Docker,
    Builtin::Apple,
    Builtin::AppBundle,
    Builtin::Command,
];

impl Extension for Builtin {
    fn descriptor(&self) -> Descriptor {
        let (id, capability) = match self {
            Self::Docker => ("docker", Capability::Collect),
            Self::Apple => ("apple", Capability::Collect),
            Self::AppBundle => ("app_bundle", Capability::Classify),
            Self::Command => ("command", Capability::Details),
        };
        Descriptor {
            id: id.into(),
            version: env!("CARGO_PKG_VERSION").into(),
            protocol_version: 1,
            source: "builtin",
            platforms: vec!["macos".into()],
            capabilities: vec![capability],
            enabled: cfg!(target_os = "macos"),
            availability: "unknown",
        }
    }

    fn execute(&self, context: &Context<'_>, request: Request<'_>) -> Result<Contribution> {
        match (self, request) {
            (Self::Docker, Request::Collect) => {
                let (runtime, containers) = containers::collect_docker(context)?;
                Ok(Contribution::Collect {
                    runtime,
                    containers,
                })
            }
            (Self::Apple, Request::Collect) => {
                let (runtime, containers) = containers::collect_apple(context)?;
                Ok(Contribution::Collect {
                    runtime,
                    containers,
                })
            }
            (Self::AppBundle, Request::Classify(processes)) => Ok(Contribution::Classify {
                proposals: processes
                    .iter()
                    .filter_map(|process| {
                        Some(ClassificationProposal {
                            identity: ProcessIdentity::observed(process)?,
                            evidence: ClassificationEvidence::AppBundle {
                                path: app_bundle(process.executable.as_deref()?)?,
                            },
                        })
                    })
                    .collect(),
            }),
            (
                Self::Command,
                Request::Details {
                    process,
                    pointer_width,
                },
            ) => {
                let identity = ProcessIdentity::observed(process)
                    .ok_or_else(|| anyhow::anyhow!("process identity unavailable"))?;
                Ok(Contribution::Details {
                    proposal: DetailProposal {
                        identity,
                        command: crate::collect::extension_command(process, pointer_width),
                    },
                })
            }
            _ => bail!("unsupported extension request"),
        }
    }
}

pub fn settings_registry() -> Result<Registry> {
    let mut registry = Registry::core();
    let contract = registry
        .definition("sampling.interval_ms")
        .expect("core sampling definition")
        .contract
        .clone();
    for extension in &BUILTINS {
        let descriptor = extension.descriptor();
        let uses = if descriptor.capabilities.contains(&Capability::Collect) {
            vec![Use {
                key: "sampling.interval_ms".into(),
                contract: contract.clone(),
            }]
        } else {
            vec![]
        };
        registry.register_extension(&descriptor.id, vec![], uses)?;
    }
    Ok(registry)
}

pub fn default_settings() -> Result<Resolved> {
    settings_registry()?.resolve(&Config::default(), &BTreeMap::new())
}

pub fn list(settings: &Resolved, containers: bool) -> Vec<Descriptor> {
    BUILTINS
        .iter()
        .map(|extension| {
            let mut descriptor = extension.descriptor();
            descriptor.enabled &= !settings
                .report()
                .disabled_extensions
                .contains_key(&descriptor.id)
                && (containers || !descriptor.capabilities.contains(&Capability::Collect));
            descriptor
        })
        .collect()
}

pub(crate) fn app_bundle(path: &Path) -> Option<PathBuf> {
    if !path.is_absolute() {
        return None;
    }
    let mut bundle = PathBuf::new();
    for component in path.components() {
        bundle.push(component);
        let name = bundle
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if (name.ends_with(".app") || name.ends_with(".app.bundle"))
            && path
                .strip_prefix(&bundle)
                .ok()
                .is_some_and(|rest| rest.starts_with("Contents"))
        {
            return Some(bundle);
        }
    }
    None
}

pub type Classifications = (
    BTreeMap<u32, PathBuf>,
    BTreeMap<u32, (PathBuf, crate::model::ExtensionAttribution)>,
    Vec<Diagnostic>,
);

pub fn classify(
    providers: &[&dyn Extension],
    context: &Context<'_>,
    processes: &[Process],
) -> Classifications {
    let (accepted, custom, diagnostics, _) = classify_with_roles(providers, context, processes);
    (accepted, custom, diagnostics)
}

pub const MAX_ROLES_PER_PROCESS: usize = 64;

pub type ClassificationFacts = (
    BTreeMap<u32, PathBuf>,
    BTreeMap<u32, (PathBuf, crate::model::ExtensionAttribution)>,
    Vec<Diagnostic>,
    BTreeMap<u32, Vec<crate::model::Role>>,
);

pub fn classify_with_roles(
    providers: &[&dyn Extension],
    context: &Context<'_>,
    processes: &[Process],
) -> ClassificationFacts {
    let mut roles: BTreeMap<u32, Vec<crate::model::Role>> = BTreeMap::new();
    let mut accepted = BTreeMap::new();
    let mut custom = BTreeMap::new();
    let mut diagnostics = Vec::new();
    for (index, (id, result)) in dispatch_many(providers, context, Request::Classify(processes))
        .into_iter()
        .enumerate()
    {
        let validation_started = Instant::now();
        let label = id
            .clone()
            .unwrap_or_else(|| format!("unidentified-{index}"));
        match result {
            Ok(Response {
                provider,
                version,
                contribution: Contribution::Classify { proposals },
            }) => {
                let mut counts = BTreeMap::new();
                let exceeded = proposals.iter().any(|proposal| {
                    if !matches!(proposal.evidence, ClassificationEvidence::Role { .. }) {
                        return false;
                    }
                    let count = counts
                        .entry(proposal.identity.pid)
                        .or_insert_with(|| roles.get(&proposal.identity.pid).map_or(0, Vec::len));
                    *count += 1;
                    *count > MAX_ROLES_PER_PROCESS
                });
                if exceeded {
                    let _ = context.finish_phase::<()>(
                        Some(&provider),
                        "classify",
                        validation_started.elapsed(),
                        Err(anyhow::anyhow!("role_limit")),
                    );
                    diagnostics.push(invalid(&provider, "classify"));
                    continue;
                }
                for proposal in proposals {
                    let process = processes.iter().find(|process| {
                        ProcessIdentity::observed(process) == Some(proposal.identity)
                    });
                    match (process, proposal.evidence) {
                        (Some(process), ClassificationEvidence::AppBundle { path })
                            if process.executable.as_deref().and_then(app_bundle).as_ref()
                                == Some(&path) =>
                        {
                            accepted.entry(proposal.identity.pid).or_insert(path);
                        }
                        (
                            Some(process),
                            ClassificationEvidence::Custom {
                                key,
                                label,
                                path,
                                basis,
                            },
                        ) if external::valid_custom(process, &key, &label, &path, &basis) => {
                            custom.entry(proposal.identity.pid).or_insert((
                                path,
                                external::evidence(
                                    provider.clone(),
                                    version.clone(),
                                    key,
                                    label,
                                    basis,
                                ),
                            ));
                        }
                        (
                            Some(process),
                            ClassificationEvidence::Role {
                                key,
                                label,
                                path,
                                basis,
                            },
                        ) if external::valid_custom(process, &key, &label, &path, &basis)
                            && !roles.get(&process.pid).is_some_and(|roles| {
                                roles
                                    .iter()
                                    .any(|role| role.provider == provider && role.key == key)
                            }) =>
                        {
                            roles
                                .entry(process.pid)
                                .or_default()
                                .push(crate::model::Role {
                                    provider: provider.clone(),
                                    version: version.clone(),
                                    key,
                                    label,
                                    path,
                                    basis,
                                });
                        }
                        _ => {
                            let _ = context.finish_phase::<()>(
                                Some(&provider),
                                "classify",
                                Duration::ZERO,
                                Err(anyhow::anyhow!("invalid classification")),
                            );
                            diagnostics.push(invalid(&provider, "classify"));
                        }
                    }
                }
                let _ = context.finish_phase(
                    Some(&provider),
                    "classify",
                    validation_started.elapsed(),
                    Ok(()),
                );
            }
            _ => diagnostics.push(invalid(&label, "classify")),
        }
    }
    (accepted, custom, diagnostics, roles)
}

fn invalid(provider: &str, capability: &str) -> Diagnostic {
    let operation_id: String = provider
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    Diagnostic::new(
        "invalid_data",
        format!("extensions.{operation_id}.{capability}"),
        format!("Extension {provider} contribution rejected; invalid data or provider unavailable"),
    )
}

pub fn details(
    provider: &dyn Extension,
    context: &Context<'_>,
    process: &Process,
    pointer_width: usize,
) -> Result<ProcessCommand> {
    let response = dispatch(
        provider,
        context,
        Request::Details {
            process,
            pointer_width,
        },
    )?;
    let id = response.provider.clone();
    let validation_started = Instant::now();
    let result = boundary(|| {
        let Contribution::Details { proposal } = response.contribution else {
            bail!("invalid detail contribution");
        };
        if ProcessIdentity::observed(process) != Some(proposal.identity) {
            bail!("detail identity mismatch");
        }
        crate::collect::validate_extension_command(proposal.command, process.executable.as_deref())
    });
    context.finish_phase(Some(&id), "details", validation_started.elapsed(), result)
}

fn dispatch_many(
    providers: &[&dyn Extension],
    context: &Context<'_>,
    request: Request<'_>,
) -> Vec<(Option<String>, Result<Response>)> {
    providers
        .chunks(4)
        .flat_map(|chunk| {
            std::thread::scope(|scope| {
                let handles: Vec<_> = chunk
                    .iter()
                    .map(|provider| {
                        scope.spawn(move || dispatch_labeled(*provider, context, request))
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|handle| {
                        handle.join().unwrap_or_else(|_| {
                            (None, Err(anyhow::anyhow!("extension worker failed")))
                        })
                    })
                    .collect::<Vec<_>>()
            })
        })
        .collect()
}

pub fn collect(providers: &[&dyn Extension], context: &Context<'_>) -> Result<crate::model::Node> {
    Ok(collect_scoped(providers, context, true)?.0)
}

pub fn collect_scoped(
    providers: &[&dyn Extension],
    context: &Context<'_>,
    include_containers: bool,
) -> Result<(crate::model::Node, crate::model::Node)> {
    let mut resources = Vec::new();
    let mut sources: Vec<(String, Runtime, Vec<Container>)> = dispatch_many(
        providers,
        context,
        if include_containers {
            Request::Collect
        } else {
            Request::CollectResources
        },
    )
    .into_iter()
    .enumerate()
    .filter_map(|(index, (id, result))| {
        let label = id
            .clone()
            .unwrap_or_else(|| format!("unidentified-{index}"));
        let validation_started = Instant::now();
        let result = result.and_then(|response| {
            if let Contribution::Collect { containers, .. }
            | Contribution::Collected { containers, .. } = &response.contribution
            {
                containers::validate_containers(containers)?;
            }
            Ok(response)
        });
        let result = context.finish_phase(
            id.as_deref(),
            "collect",
            validation_started.elapsed(),
            result,
        );
        match result {
            Ok(Response {
                contribution:
                    Contribution::Collect {
                        runtime,
                        containers,
                    },
                ..
            }) => Some((label, runtime, containers)),
            Ok(Response {
                contribution:
                    Contribution::Collected {
                        runtime,
                        containers,
                        resources: items,
                    },
                ..
            }) => {
                resources.extend(items);
                runtime.map(|runtime| (label, runtime, containers))
            }
            _ if !include_containers
                || !boundary(|| Ok(providers[index].collects_containers())).unwrap_or(false) =>
            {
                None
            }
            _ => Some((
                label.clone(),
                Runtime {
                    name: label.clone(),
                    context: None,
                    endpoint_ref: None,
                    daemon_id: None,
                    status: "unavailable".into(),
                    inventory_complete: false,
                    collection_ms: 0,
                    unavailable: vec![invalid(&label, "collect")],
                },
                vec![],
            )),
        }
    })
    .collect();
    let mut memory = 0u64;
    let mut cpu = 0.0;
    for (provider, runtime, containers) in &mut sources {
        let validation_started = Instant::now();
        let next_memory = containers
            .iter()
            .filter(|container| !container.is_inactive())
            .filter_map(|container| container.memory_bytes)
            .try_fold(memory, |total, value| total.checked_add(value));
        let next_cpu = cpu
            + containers
                .iter()
                .filter(|container| !container.is_inactive())
                .filter_map(|container| container.cpu_percent)
                .sum::<f64>();
        if let Some(next_memory) = next_memory
            && next_cpu.is_finite()
        {
            memory = next_memory;
            cpu = next_cpu;
        } else {
            let _ = context.finish_phase::<()>(
                Some(provider),
                "collect",
                Duration::ZERO,
                Err(anyhow::anyhow!("invalid aggregate")),
            );
            runtime.inventory_complete = false;
            runtime.status = "unavailable".into();
            runtime.unavailable.push(invalid(provider, "collect"));
            containers.clear();
        }
        let _ = context.finish_phase(
            Some(provider),
            "collect",
            validation_started.elapsed(),
            Ok(()),
        );
    }
    context.cancel.check()?;
    let containers = containers::build_all(
        sources
            .into_iter()
            .map(|(_, runtime, containers)| (runtime, containers))
            .collect(),
    )?;
    Ok((containers, resources::build(resources)))
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod protocol_tests;
