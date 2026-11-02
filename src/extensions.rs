use crate::{
    containers::{self, Container, Runtime},
    model::{Diagnostic, Process, ProcessCommand},
    settings::{Config, Registry, Resolved, Use},
    snapshot::Cancellation,
};
use anyhow::{Result, bail};
use serde::Serialize;
use std::{
    cell::Cell,
    collections::BTreeMap,
    panic::{self, AssertUnwindSafe},
    path::{Path, PathBuf},
    sync::Once,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Collect,
    Classify,
    Details,
}

#[derive(Serialize)]
pub struct Descriptor {
    pub id: &'static str,
    pub version: &'static str,
    pub protocol_version: u32,
    pub source: &'static str,
    pub platforms: Vec<&'static str>,
    pub capabilities: Vec<Capability>,
    pub enabled: bool,
    pub availability: &'static str,
}

pub struct Context<'a> {
    pub settings: &'a Resolved,
    pub cancel: &'a Cancellation,
    pub deadline: Instant,
}

impl<'a> Context<'a> {
    pub fn new(settings: &'a Resolved, cancel: &'a Cancellation) -> Self {
        Self {
            settings,
            cancel,
            deadline: Instant::now() + Duration::from_secs(10),
        }
    }
    pub fn check(&self) -> Result<()> {
        self.cancel.check()?;
        if Instant::now() >= self.deadline {
            bail!("extension deadline exceeded");
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
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

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ClassificationEvidence {
    AppBundle { path: PathBuf },
}

#[derive(Clone, Debug, Serialize)]
pub struct ClassificationProposal {
    pub identity: ProcessIdentity,
    pub evidence: ClassificationEvidence,
}

#[derive(Clone, Debug, Serialize)]
pub struct DetailProposal {
    pub identity: ProcessIdentity,
    pub command: ProcessCommand,
}

pub enum Request<'a> {
    Collect,
    Classify(&'a [Process]),
    Details {
        process: &'a Process,
        pointer_width: usize,
    },
}

impl Request<'_> {
    fn capability(&self) -> Capability {
        match self {
            Self::Collect => Capability::Collect,
            Self::Classify(_) => Capability::Classify,
            Self::Details { .. } => Capability::Details,
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(tag = "capability", rename_all = "snake_case")]
pub enum Contribution {
    Collect {
        runtime: Runtime,
        containers: Vec<Container>,
    },
    Classify {
        proposals: Vec<ClassificationProposal>,
    },
    Details {
        proposal: DetailProposal,
    },
}

impl Contribution {
    fn capability(&self) -> Capability {
        match self {
            Self::Collect { .. } => Capability::Collect,
            Self::Classify { .. } => Capability::Classify,
            Self::Details { .. } => Capability::Details,
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
) -> (Option<&'static str>, Result<Response>) {
    let mut provider = None;
    let result = boundary(|| {
        context.check()?;
        let descriptor = extension.descriptor();
        provider = Some(descriptor.id);
        let capability = request.capability();
        if !descriptor.enabled
            || context
                .settings
                .report()
                .disabled_extensions
                .contains_key(descriptor.id)
            || !descriptor.capabilities.contains(&capability)
        {
            bail!("extension capability is disabled or unsupported");
        }
        let contribution = extension
            .execute(context, request)
            .map_err(|_| anyhow::anyhow!("extension failed"))?;
        context.check()?;
        if contribution.capability() != capability {
            bail!("extension returned an unexpected capability");
        }
        Ok(Response {
            provider: descriptor.id.into(),
            version: descriptor.version.into(),
            contribution,
        })
    });
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
            id,
            version: "1",
            protocol_version: 1,
            source: "builtin",
            platforms: vec!["macos"],
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
        registry.register_extension(descriptor.id, vec![], uses)?;
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
                .contains_key(descriptor.id)
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

pub fn classify(
    providers: &[&dyn Extension],
    context: &Context<'_>,
    processes: &[Process],
) -> (BTreeMap<u32, PathBuf>, Vec<Diagnostic>) {
    let mut accepted = BTreeMap::new();
    let mut diagnostics = Vec::new();
    for (index, provider) in providers.iter().enumerate() {
        let (id, result) = dispatch_labeled(*provider, context, Request::Classify(processes));
        let label = id
            .map(str::to_owned)
            .unwrap_or_else(|| format!("unidentified-{index}"));
        match result {
            Ok(Response {
                contribution: Contribution::Classify { proposals },
                ..
            }) => {
                for proposal in proposals {
                    let valid = processes
                        .iter()
                        .find(|process| {
                            ProcessIdentity::observed(process) == Some(proposal.identity)
                        })
                        .and_then(|process| {
                            let ClassificationEvidence::AppBundle { path } = proposal.evidence;
                            (process.executable.as_deref().and_then(app_bundle).as_ref()
                                == Some(&path))
                            .then_some(path)
                        });
                    if let Some(path) = valid {
                        accepted.entry(proposal.identity.pid).or_insert(path);
                    } else {
                        diagnostics.push(invalid(&label, "classify"));
                    }
                }
            }
            _ => diagnostics.push(invalid(&label, "classify")),
        }
    }
    (accepted, diagnostics)
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
    let Contribution::Details { proposal } = response.contribution else {
        bail!("invalid detail contribution");
    };
    if ProcessIdentity::observed(process) != Some(proposal.identity) {
        bail!("detail identity mismatch");
    }
    crate::collect::validate_extension_command(proposal.command, process.executable.as_deref())
}

pub fn collect(providers: &[&dyn Extension], context: &Context<'_>) -> Result<crate::model::Node> {
    let sources = std::thread::scope(|scope| {
        let handles: Vec<_> = providers
            .iter()
            .map(|provider| scope.spawn(|| dispatch_labeled(*provider, context, Request::Collect)))
            .collect();
        handles
            .into_iter()
            .enumerate()
            .map(|(index, handle)| {
                let (id, result) = handle
                    .join()
                    .unwrap_or_else(|_| (None, Err(anyhow::anyhow!("extension worker failed"))));
                let label = id
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("unidentified-{index}"));
                match result {
                    Ok(Response {
                        contribution:
                            Contribution::Collect {
                                runtime,
                                containers,
                            },
                        ..
                    }) if containers::validate_containers(&containers).is_ok() => {
                        (runtime, containers)
                    }
                    _ => (
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
                    ),
                }
            })
            .collect()
    });
    context.cancel.check()?;
    containers::build_all(sources)
}

#[cfg(test)]
mod tests;
