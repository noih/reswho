use super::{
    state::{HOST_VERSION, Manifest, PROTOCOL_VERSION, State},
    *,
};
use crate::model::{DetailField, ExtensionAttribution, ExtensionDetails};
use anyhow::{Result, bail};
#[cfg(test)]
use serde_json::Value;
use serde_json::json;

pub struct External {
    pub manifest: Manifest,
}

pub fn installed(state: &State, settings: &Resolved) -> Vec<External> {
    state
        .installed
        .iter()
        .filter(|entry| {
            !entry.removed
                && entry.problem.is_none()
                && entry.enabled
                && entry.manifest.incompatibility().is_none()
                && !settings
                    .report()
                    .disabled_extensions
                    .contains_key(&entry.manifest.id)
        })
        .map(|entry| External {
            manifest: entry.manifest.clone(),
        })
        .collect()
}

type WireResponse = super::protocol::Response;

fn observed(process: &Process) -> Result<super::protocol::ObservedProcess> {
    Ok(super::protocol::ObservedProcess {
        pid: process.pid,
        started_unix_us: process
            .started_unix_us
            .ok_or_else(|| anyhow::anyhow!("identity_unavailable"))?,
        ppid: process.ppid,
        uid: process.uid,
        name: process.name.clone(),
        executable: process.executable.clone(),
        cwd: process.cwd.clone(),
    })
}

impl Extension for External {
    fn collects_containers(&self) -> bool {
        self.manifest
            .collect_scopes
            .iter()
            .any(|scope| scope == "containers")
    }
    fn descriptor(&self) -> Descriptor {
        Descriptor {
            id: self.manifest.id.clone(),
            version: self.manifest.version.clone(),
            protocol_version: self.manifest.protocol_version,
            source: "external",
            platforms: self.manifest.platforms.clone(),
            capabilities: self.manifest.known_capabilities(),
            enabled: self.manifest.incompatibility().is_none(),
            availability: "unknown",
        }
    }
    fn execute(&self, context: &Context<'_>, request: Request<'_>) -> Result<Contribution> {
        if self.manifest.incompatibility().is_some() {
            bail!("incompatible extension");
        }
        let mut settings = BTreeMap::new();
        for key in self.manifest.uses.iter().map(|usage| &usage.key).chain(
            self.manifest
                .definitions
                .iter()
                .map(|definition| &definition.key),
        ) {
            if let Some(value) = context.settings.get(key) {
                settings.insert(key.clone(), value.clone());
            }
        }
        let mut input = super::protocol::Request {
            protocol_version: PROTOCOL_VERSION,
            host: super::protocol::Host {
                version: HOST_VERSION.into(),
                protocol_version: PROTOCOL_VERSION,
                features: super::protocol::FEATURES
                    .iter()
                    .map(|feature| (*feature).into())
                    .collect(),
            },
            provider: self.manifest.id.clone(),
            version: self.manifest.version.clone(),
            settings,
            capability: request.capability(),
            collect_scopes: vec![],
            processes: None,
            process: None,
        };
        match request {
            Request::Collect | Request::CollectResources => {
                input.collect_scopes = self
                    .manifest
                    .collect_scopes
                    .iter()
                    .filter(|scope| {
                        matches!(request, Request::Collect) || scope.as_str() == "resources"
                    })
                    .cloned()
                    .collect();
                if input.collect_scopes.is_empty() {
                    bail!("unsupported_collect_scope");
                }
            }
            Request::Classify(processes) => {
                if processes.len() > 16384 {
                    bail!("process request limit exceeded");
                }
                input.processes = Some(
                    processes
                        .iter()
                        .filter(|process| process.started_unix_us.is_some())
                        .map(observed)
                        .collect::<Result<_>>()?,
                );
            }
            Request::Details { process, .. } => input.process = Some(observed(process)?),
        }
        let bytes = serde_json::to_vec(&input)?;
        let output = crate::containers::command::run_bounded(
            &self.manifest.executable,
            &self.manifest.args,
            Some(&bytes),
            context.cancel,
            context.deadline,
        )?;
        if output.issue.is_some() {
            bail!("extension process failed; stderr withheld");
        }
        let response: WireResponse = serde_json::from_slice(&output.stdout)
            .map_err(|_| anyhow::anyhow!("invalid extension response"))?;
        if response.protocol_version != PROTOCOL_VERSION
            || response.provider != self.manifest.id
            || response.version != self.manifest.version
        {
            bail!("extension response version or provider mismatch");
        }
        if !super::protocol::supported(&response.required_features) {
            bail!("unsupported_required_feature");
        }
        let contribution = match response.contribution {
            super::protocol::Contribution::Collect {
                runtime,
                containers,
                resources,
            } => {
                if !matches!(request, Request::Collect | Request::CollectResources)
                    || (runtime.is_some() || containers.is_some())
                        && !input
                            .collect_scopes
                            .iter()
                            .any(|scope| scope == "containers")
                    || resources.is_some()
                        && !input
                            .collect_scopes
                            .iter()
                            .any(|scope| scope == "resources")
                    || runtime.is_some() != containers.is_some()
                    || runtime.is_none() && resources.is_none()
                {
                    bail!("unexpected_collect_scope");
                }
                let resources = super::resources::convert(
                    &self.manifest.id,
                    &self.manifest.version,
                    resources.unwrap_or_default(),
                )?;
                Contribution::Collected {
                    runtime: runtime.map(Into::into),
                    containers: containers
                        .unwrap_or_default()
                        .into_iter()
                        .map(Into::into)
                        .collect(),
                    resources,
                }
            }
            super::protocol::Contribution::Classify { proposals } => Contribution::Classify {
                proposals: proposals.into_iter().map(Into::into).collect(),
            },
            super::protocol::Contribution::ParsedDetails { identity, fields } => {
                Contribution::ParsedDetails {
                    identity: identity.into(),
                    fields: fields.into_iter().map(Into::into).collect(),
                }
            }
        };
        match contribution {
            Contribution::Collected {
                runtime,
                mut containers,
                resources,
            } => {
                let Some(mut runtime) = runtime else {
                    return Ok(Contribution::Collected {
                        runtime: None,
                        containers,
                        resources,
                    });
                };
                if !matches!(runtime.status.as_str(), "available" | "unavailable")
                    || runtime.name != self.manifest.id
                    || containers.len() > 4096
                    || runtime.endpoint_ref.is_none()
                {
                    bail!("invalid extension runtime identity");
                }
                containers::validate_containers(&containers)?;
                for container in &mut containers {
                    if (container.is_inactive()
                        && (container.cpu_percent.is_some()
                            || container.memory_bytes.is_some()
                            || container.uptime_seconds.is_some()))
                        || container.id.is_empty()
                        || container.id.len() > 256
                        || container.runtime != runtime.name
                        || Some(&container.endpoint_ref) != runtime.endpoint_ref.as_ref()
                        || container
                            .cpu_percent
                            .is_some_and(|value| !value.is_finite() || value < 0.0)
                    {
                        bail!("invalid extension container");
                    }
                    scrub(&mut container.unavailable, &self.manifest.id);
                }
                let cpu_total: f64 = containers
                    .iter()
                    .filter_map(|container| container.cpu_percent)
                    .sum();
                let memory_total = containers
                    .iter()
                    .filter_map(|container| container.memory_bytes)
                    .try_fold(0u64, |total, value| total.checked_add(value));
                if !cpu_total.is_finite() || memory_total.is_none() {
                    bail!("extension totals overflow");
                }
                scrub(&mut runtime.unavailable, &self.manifest.id);
                Ok(Contribution::Collected {
                    runtime: Some(runtime),
                    containers,
                    resources,
                })
            }
            Contribution::Classify { proposals } => {
                let Request::Classify(processes) = request else {
                    bail!("unexpected classify response");
                };
                if proposals.len() > 16384 {
                    bail!("classification proposal limit exceeded");
                }
                let mut roles = HashSet::new();
                for proposal in &proposals {
                    if let ClassificationEvidence::Role { key, .. } = &proposal.evidence
                        && !roles.insert((proposal.identity.pid, key.clone()))
                    {
                        bail!("duplicate_role");
                    }
                    let process = processes
                        .iter()
                        .find(|process| {
                            ProcessIdentity::observed(process) == Some(proposal.identity)
                        })
                        .ok_or_else(|| anyhow::anyhow!("classification identity mismatch"))?;
                    match &proposal.evidence {
                        ClassificationEvidence::Custom {
                            key,
                            label,
                            path,
                            basis,
                        }
                        | ClassificationEvidence::Role {
                            key,
                            label,
                            path,
                            basis,
                        } if valid_custom(process, key, label, path, basis) => {}
                        _ => bail!("external classification requires observed custom evidence"),
                    }
                }
                Ok(Contribution::Classify { proposals })
            }
            Contribution::ParsedDetails {
                identity,
                mut fields,
            } => {
                let Request::Details { process, .. } = request else {
                    bail!("unexpected details response");
                };
                if ProcessIdentity::observed(process) != Some(identity) {
                    bail!("details identity mismatch");
                }
                validate_fields(&mut fields, &self.manifest.id)?;
                Ok(Contribution::ParsedDetails { identity, fields })
            }
            _ => bail!("unsupported external contribution"),
        }
    }
}

fn scrub(diagnostics: &mut [Diagnostic], provider: &str) {
    for diagnostic in diagnostics {
        *diagnostic = super::invalid(provider, "collect");
    }
}

pub fn valid_custom(process: &Process, key: &str, label: &str, path: &Path, basis: &str) -> bool {
    let observed = match basis {
        "cwd" => process.cwd.as_deref(),
        "executable" => process.executable.as_deref(),
        _ => None,
    };
    !key.is_empty()
        && key.len() <= 128
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
        && !label.is_empty()
        && label.len() <= 128
        && path.is_absolute()
        && observed == Some(path)
}

fn validate_fields(fields: &mut [DetailField], provider: &str) -> Result<()> {
    if fields.len() > 64 {
        bail!("parsed detail field limit exceeded");
    }
    let mut keys = HashSet::new();
    for field in fields {
        if !field.key.starts_with(&format!("extensions.{provider}."))
            || field.key.len() <= format!("extensions.{provider}.").len()
            || field.key.len() > 256
            || !field
                .key
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
            || field.label.is_empty()
            || field.label.len() > 128
            || !keys.insert(field.key.clone())
            || field.value.is_array()
            || field.value.is_object()
            || field
                .value
                .as_str()
                .is_some_and(|value| value.len() > 16384)
        {
            bail!("invalid parsed detail field");
        }
        let local_key = field
            .key
            .strip_prefix(&format!("extensions.{provider}."))
            .expect("validated field namespace");
        if field.sensitive || crate::redaction::sensitive_option(local_key) {
            field.sensitive = true;
            field.value = json!("[filtered]");
        }
    }
    Ok(())
}

pub fn attach_details(
    providers: &[&dyn Extension],
    context: &Context<'_>,
    process: &mut Process,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for (index, (id, result)) in super::dispatch_many(
        providers,
        context,
        Request::Details {
            process,
            pointer_width: 8,
        },
    )
    .into_iter()
    .enumerate()
    {
        let validation_started = Instant::now();
        let result = result.and_then(|response| {
            let Contribution::ParsedDetails {
                identity,
                mut fields,
            } = response.contribution
            else {
                bail!("invalid parsed details");
            };
            if ProcessIdentity::observed(process) != Some(identity) {
                bail!("parsed detail identity mismatch");
            }
            validate_fields(&mut fields, &response.provider)?;
            Ok(ExtensionDetails {
                provider: response.provider,
                version: response.version,
                fields,
            })
        });
        match context.finish_phase(
            id.as_deref(),
            "details",
            validation_started.elapsed(),
            result,
        ) {
            Ok(details) => process.extension_details.push(details),
            Err(_) => diagnostics.push(super::invalid(
                id.as_deref().unwrap_or(&format!("unidentified-{index}")),
                "details",
            )),
        }
    }
    diagnostics
}

pub fn evidence(
    provider: String,
    version: String,
    key: String,
    label: String,
    basis: String,
) -> ExtensionAttribution {
    ExtensionAttribution {
        provider,
        version,
        key,
        label,
        basis,
    }
}

#[cfg(test)]
mod tests;
