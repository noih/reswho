use super::{
    state::{HOST_VERSION, Manifest, PROTOCOL_VERSION, State},
    *,
};
use crate::model::{DetailField, ExtensionAttribution, ExtensionDetails};
use anyhow::{Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};

pub struct External {
    pub manifest: Manifest,
}

pub fn installed(state: &State, settings: &Resolved) -> Vec<External> {
    state
        .installed
        .iter()
        .filter(|entry| {
            !entry.removed
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireResponse {
    protocol_version: u32,
    provider: String,
    version: String,
    contribution: Contribution,
}

fn observed(process: &Process) -> Value {
    json!({"pid":process.pid,"started_unix_us":process.started_unix_us,"ppid":process.ppid,"uid":process.uid,"name":process.name,"executable":process.executable,"cwd":process.cwd})
}

impl Extension for External {
    fn descriptor(&self) -> Descriptor {
        Descriptor {
            id: self.manifest.id.clone(),
            version: self.manifest.version.clone(),
            protocol_version: self.manifest.protocol_version,
            source: "external",
            platforms: self.manifest.platforms.clone(),
            capabilities: self.manifest.capabilities.clone(),
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
                settings.insert(key, value);
            }
        }
        let mut input = json!({"protocol_version":PROTOCOL_VERSION,"host":{"version":HOST_VERSION,"protocol_version":PROTOCOL_VERSION},"provider":self.manifest.id,"version":self.manifest.version,"settings":settings});
        match request {
            Request::Collect => input["capability"] = json!("collect"),
            Request::Classify(processes) => {
                if processes.len() > 16384 {
                    bail!("process request limit exceeded");
                }
                input["capability"] = json!("classify");
                input["processes"] = processes
                    .iter()
                    .filter(|process| process.started_unix_us.is_some())
                    .map(observed)
                    .collect();
            }
            Request::Details { process, .. } => {
                input["capability"] = json!("details");
                input["process"] = observed(process);
            }
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
        match response.contribution {
            Contribution::Collect {
                mut runtime,
                mut containers,
            } => {
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
                Ok(Contribution::Collect {
                    runtime,
                    containers,
                })
            }
            Contribution::Classify { proposals } => {
                let Request::Classify(processes) = request else {
                    bail!("unexpected classify response");
                };
                if proposals.len() > 16384 {
                    bail!("classification proposal limit exceeded");
                }
                for proposal in &proposals {
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
        match context.finish(id.as_deref(), result) {
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
