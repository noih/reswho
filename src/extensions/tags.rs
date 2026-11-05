use super::{ClassificationEvidence, Context, Extension, ProcessIdentity, Request};
use crate::model::{Diagnostic, Process};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Instant;

pub const MAX_PER_PROCESS: usize = 64;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tag {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_resource: Option<Box<crate::model::HostResource>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supplementary_to: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_chain: Vec<ProcessEvidence>,
    pub dimension: String,
    pub id: String,
    pub label: String,
    pub path: Option<PathBuf>,
    pub basis: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projection: Option<Box<super::placement::Projection>>,
    #[serde(skip)]
    pub(crate) assignment: Option<Box<crate::extensions::host::assignment::Assignment>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessEvidence {
    pub identity: ProcessIdentity,
    pub executable: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selector {
    #[serde(skip)]
    pub(crate) allowed_sources: Vec<String>,
    pub dimension: String,
    pub tag_id: Option<String>,
    pub source: Option<String>,
}

impl Selector {
    fn matches(&self, source: &str, tag: &Tag) -> bool {
        self.dimension == tag.dimension
            && (self.allowed_sources.is_empty()
                || self.allowed_sources.iter().any(|id| id == source))
            && self.tag_id.as_ref().is_none_or(|id| id == &tag.id)
            && self.source.as_ref().is_none_or(|id| id == source)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Add { tag: Tag },
    Exclude { selector: Selector },
    Withdraw { proposal_id: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proposal {
    pub id: String,
    pub identity: ProcessIdentity,
    #[serde(rename = "change")]
    pub operation: Operation,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contract {
    pub produces: Vec<String>,
    pub overrides: Vec<Override>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Override {
    pub dimension: String,
    pub sources: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Recorded {
    pub source: String,

    #[serde(serialize_with = "public_proposal")]
    pub proposal: Proposal,
}

fn public_proposal<S: serde::Serializer>(
    proposal: &Proposal,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let mut public = proposal.clone();
    if let Operation::Add { tag } = &mut public.operation {
        tag.projection = None;
    }
    public.serialize(serializer)
}

fn public_tag<S: serde::Serializer>(tag: &Tag, serializer: S) -> Result<S::Ok, S::Error> {
    let mut public = tag.clone();
    public.projection = None;
    public.serialize(serializer)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Effective {
    pub source: String,
    #[serde(serialize_with = "public_tag")]
    pub tag: Tag,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Decision {
    pub source: String,
    pub proposal_id: String,
    pub status: String,
    pub by_source: Option<String>,
    pub reason: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Report {
    pub effective: Vec<Effective>,
    pub proposals: Vec<Recorded>,
    pub decisions: Vec<Decision>,
    pub diagnostics: Vec<Diagnostic>,
}

fn text(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}

fn identifier(value: &str) -> bool {
    text(value, 256)
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_.:-/".contains(c))
}

fn source_id(value: &str) -> bool {
    text(value, 64)
        && value
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

fn dimension(value: &str, provider: &str) -> bool {
    text(value, 128)
        && (matches!(
            value,
            "project" | "app" | "bin" | "runtime" | "role" | "version" | "name" | "work"
        ) || value
            .strip_prefix(&format!("extensions.{provider}."))
            .is_some_and(identifier))
}

fn override_dimension(value: &str) -> bool {
    text(value, 128)
        && (matches!(
            value,
            "project" | "app" | "bin" | "runtime" | "role" | "version" | "name" | "work"
        ) || value.strip_prefix("extensions.").is_some_and(|rest| {
            rest.split_once('.')
                .is_some_and(|(source, name)| source_id(source) && identifier(name))
        }))
}

fn tag_id(dimension: &str, value: &str, provider: &str) -> bool {
    identifier(value)
        && (value.starts_with(&format!("extensions.{provider}."))
            || value.starts_with(&format!("{dimension}:")))
}

impl Contract {
    pub fn validate(&self, provider: &str) -> Result<()> {
        if self.produces.len() > 32
            || self.overrides.len() > 32
            || self.produces.iter().any(|d| !dimension(d, provider))
            || self.overrides.iter().any(|rule| {
                !override_dimension(&rule.dimension)
                    || rule.sources.is_empty()
                    || rule.sources.len() > 128
                    || rule.sources.iter().any(|s| !source_id(s))
            })
        {
            bail!("invalid_tag_contract");
        }
        Ok(())
    }
}

pub fn validate(
    proposals: &[Proposal],
    processes: &[Process],
    source: &str,
    contract: Option<&Contract>,
) -> Result<()> {
    validate_registered(proposals, processes, source, contract, &[])
}

pub(crate) fn validate_registered(
    proposals: &[Proposal],
    processes: &[Process],
    source: &str,
    contract: Option<&Contract>,
    ownership: &[super::placement::Ownership],
) -> Result<()> {
    if proposals.len() > 16384 {
        bail!("tag_proposal_limit");
    }
    let mut ids = BTreeSet::new();
    let mut counts = BTreeMap::new();
    for proposal in proposals {
        let process = processes
            .iter()
            .find(|p| ProcessIdentity::observed(p) == Some(proposal.identity))
            .ok_or_else(|| anyhow::anyhow!("tag_identity_mismatch"))?;
        if !identifier(&proposal.id) || !ids.insert((proposal.identity.pid, &proposal.id)) {
            bail!("duplicate_tag_proposal");
        }
        let count = counts.entry(process.pid).or_insert(0);
        *count += 1;
        if *count > MAX_PER_PROCESS * 4 {
            bail!("tag_proposal_limit");
        }
        match &proposal.operation {
            Operation::Add { tag } => {
                if !dimension(&tag.dimension, source)
                    || !tag_id(&tag.dimension, &tag.id, source)
                    || !text(&tag.label, 128)
                    || !text(&tag.basis, 128)
                    || tag.path.as_ref().is_some_and(|p| {
                        p.as_os_str().len() > 4096
                            || !p.is_absolute()
                            || p.to_string_lossy().chars().any(char::is_control)
                    })
                {
                    bail!("invalid_tag");
                }
                validate_chain(tag, process, processes)?;
                if let Some(resource) = &tag.host_resource
                    && (!text(&resource.kind, 64)
                        || !text(&resource.label, 128)
                        || !text(&resource.confidence, 64)
                        || resource.scope.as_ref().is_some_and(|s| !text(s, 1024))
                        || resource.evidence.len() > 65
                        || resource.evidence.iter().any(|e| !text(e, 4096))
                        || resource.diagnostics.len() > 16
                        || resource.diagnostics.iter().any(|d| {
                            !matches!(d.code.as_str(), "invalid_data" | "unavailable")
                                || !text(&d.operation, 128)
                                || !text(&d.message, 4096)
                        })
                        || resource
                            .configuration
                            .as_ref()
                            .is_some_and(|c| !text(&c.scope, 64))
                        || resource.responsible.as_ref().is_some_and(|e| {
                            !e.executable.is_absolute()
                                || !text(&e.executable.to_string_lossy(), 4096)
                                || !processes.iter().any(|p| {
                                    ProcessIdentity::observed(p) == Some(e.identity)
                                        && p.executable.as_ref() == Some(&e.executable)
                                        && p.uid.is_some()
                                        && p.uid == process.uid
                                        && p.started_unix_us <= process.started_unix_us
                                })
                        }))
                {
                    bail!("invalid_host_resource");
                }
                let versioned = tag.dimension == "version";
                if versioned
                    != (tag.subject.as_deref().is_some_and(|s| text(s, 128))
                        && tag.value.as_deref().is_some_and(|v| text(v, 128)))
                    || !versioned && (tag.subject.is_some() || tag.value.is_some())
                    || matches!(tag.dimension.as_str(), "version" | "name" | "work")
                        && tag.path.is_some()
                {
                    bail!("invalid_typed_tag");
                }
                if let Some(contract) = contract {
                    if !matches!(tag.basis.as_str(), "cwd" | "executable" | "observed") {
                        bail!("invalid_tag_basis");
                    }
                    if tag.dimension == "bin" && tag.basis != "executable" {
                        bail!("bin_requires_executable");
                    }
                    if !contract.produces.contains(&tag.dimension) {
                        bail!("undeclared_tag_dimension");
                    }
                    if matches!(tag.dimension.as_str(), "project" | "app")
                        || tag.path.is_some()
                            && ownership.iter().any(|o| o.dimension == tag.dimension)
                    {
                        let root = tag
                            .path
                            .as_ref()
                            .ok_or_else(|| anyhow::anyhow!("ownership_root_required"))?;
                        let observed = match tag.basis.as_str() {
                            "cwd" => process.cwd.as_ref(),
                            "executable" => process.executable.as_ref(),
                            _ => None,
                        }
                        .ok_or_else(|| anyhow::anyhow!("ownership_basis_required"))?;
                        let root = root
                            .canonicalize()
                            .map_err(|_| anyhow::anyhow!("ownership_root_unavailable"))?;
                        let observed = observed
                            .canonicalize()
                            .map_err(|_| anyhow::anyhow!("ownership_evidence_unavailable"))?;
                        if (!root.is_dir()
                            && (matches!(tag.dimension.as_str(), "project" | "app")
                                || root != observed))
                            || root.parent().is_none()
                            || !observed.starts_with(&root)
                            || observed
                                .components()
                                .count()
                                .saturating_sub(root.components().count())
                                > 128
                        {
                            bail!("ownership_root_outside_evidence");
                        }
                        if tag.dimension == "app"
                            && (tag.basis != "executable"
                                || crate::extensions::host::bundles::app_bundle(&observed).as_ref()
                                    != Some(&root))
                        {
                            bail!("app_bundle_lineage_required");
                        }
                    } else if let Some(path) = &tag.path
                        && tag.evidence_chain.is_empty()
                    {
                        let observed = match tag.basis.as_str() {
                            "cwd" => process.cwd.as_ref(),
                            "executable" => process.executable.as_ref(),
                            _ => None,
                        };
                        if observed != Some(path) {
                            bail!("tag_path_not_observed");
                        }
                    }
                    if tag.dimension == "bin" && tag.path.is_none() {
                        bail!("bin_path_required");
                    }
                }
            }
            Operation::Exclude { selector } => {
                if !override_dimension(&selector.dimension)
                    || selector.tag_id.is_none() && selector.source.is_none()
                    || selector.tag_id.as_ref().is_some_and(|id| !identifier(id))
                    || selector.source.as_ref().is_some_and(|id| !source_id(id))
                {
                    bail!("invalid_tag_selector");
                }
                if let Some(contract) = contract
                    && !contract.overrides.iter().any(|rule| {
                        rule.dimension == selector.dimension
                            && selector
                                .source
                                .as_ref()
                                .is_none_or(|target| rule.sources.contains(target))
                    })
                {
                    bail!("undeclared_tag_override");
                }
            }
            Operation::Withdraw { proposal_id } => {
                if !identifier(proposal_id)
                    || !proposals.iter().any(|target| {
                        target.id == *proposal_id
                            && target.identity == proposal.identity
                            && !matches!(target.operation, Operation::Withdraw { .. })
                    })
                {
                    bail!("invalid_tag_withdrawal");
                }
            }
        }
    }
    Ok(())
}

fn validate_chain(tag: &Tag, process: &Process, processes: &[Process]) -> Result<()> {
    if tag.supplementary_to.is_none() && tag.evidence_chain.is_empty() {
        return Ok(());
    }
    if !tag
        .supplementary_to
        .as_deref()
        .is_some_and(override_dimension)
        || tag.projection.is_some()
        || tag.assignment.is_some()
        || tag.evidence_chain.len() < 2
        || tag.evidence_chain.len() > processes.len()
        || tag.evidence_chain.first().map(|e| e.identity) != ProcessIdentity::observed(process)
    {
        bail!("invalid_supplementary_tag");
    }
    let mut seen = BTreeSet::new();
    let mut child: Option<&Process> = None;
    for evidence in &tag.evidence_chain {
        let observed = processes
            .iter()
            .find(|p| p.pid == evidence.identity.pid)
            .ok_or_else(|| anyhow::anyhow!("invalid_evidence_chain"))?;
        if !seen.insert(observed.pid)
            || ProcessIdentity::observed(observed) != Some(evidence.identity)
            || observed.executable.as_ref() != Some(&evidence.executable)
            || !evidence.executable.is_absolute()
            || evidence.executable.as_os_str().len() > 4096
            || evidence
                .executable
                .to_string_lossy()
                .chars()
                .any(char::is_control)
            || child.is_some_and(|child| {
                child.ppid != Some(observed.pid)
                    || observed.pid <= 1
                    || observed.uid.is_none()
                    || child.uid != observed.uid
                    || observed.started_unix_us > child.started_unix_us
            })
        {
            bail!("invalid_evidence_chain");
        }
        child = Some(observed);
    }
    if tag.path.as_ref().is_none_or(|path| {
        !tag.evidence_chain
            .last()
            .is_some_and(|e| e.executable.starts_with(path))
    }) {
        bail!("invalid_evidence_root");
    }
    Ok(())
}

pub fn path_id(dimension: &str, path: &std::path::Path) -> String {
    canonical_id(
        dimension,
        &path.canonicalize().unwrap_or_else(|_| path.to_path_buf()),
    )
}

pub(crate) fn canonical_id(dimension: &str, canonical: &std::path::Path) -> String {
    format!(
        "{dimension}:{}",
        crate::extensions::host::stable_ref(dimension, &canonical.to_string_lossy())
            .trim_start_matches("group:")
    )
}

pub(crate) fn ownership_key(dimension: &str, id: &str, path: &std::path::Path) -> String {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    format!("{dimension}\0{id}\0{}", canonical.to_string_lossy())
}

pub(crate) fn ownership_ref(dimension: &str, id: &str, path: &std::path::Path) -> String {
    let category = match dimension {
        "app" => "applications",
        "project" => "projects",
        _ => "executable",
    };
    crate::extensions::host::stable_ref(category, &ownership_key(dimension, id, path))
}

fn exclusive(dimension: &str) -> bool {
    matches!(
        dimension,
        "app" | "project" | "bin" | "version" | "name" | "work"
    )
}

fn withdrawn(records: &[Recorded], index: usize) -> bool {
    records.iter().any(|record| {
        record.source == records[index].source
            && matches!(&record.proposal.operation, Operation::Withdraw { proposal_id } if proposal_id == &records[index].proposal.id)
    })
}

fn contradictory(records: &[Recorded], ownership: &[String]) -> bool {
    let active: Vec<_> = records
        .iter()
        .enumerate()
        .filter(|(i, _)| !withdrawn(records, *i))
        .map(|(_, record)| record)
        .collect();
    for (i, left) in active.iter().enumerate() {
        if let Operation::Add { tag } = &left.proposal.operation {
            for (j, right) in active.iter().enumerate() {
                if i == j {
                    continue;
                }
                match &right.proposal.operation {
                    Operation::Add { tag: other }
                        if tag.dimension == other.dimension
                            && ((exclusive(&tag.dimension)
                                || ownership.contains(&tag.dimension))
                                && tag.id != other.id
                                || tag.id == other.id
                                    && (tag.label != other.label
                                        || tag.path != other.path
                                        || tag.subject != other.subject
                                        || tag.value != other.value)) =>
                    {
                        return true;
                    }
                    Operation::Exclude { selector } if selector.matches(&left.source, tag) => {
                        return true;
                    }
                    _ => {}
                }
            }
        }
    }
    false
}

pub fn resolve(records: Vec<Recorded>, priority: &[String]) -> Report {
    resolve_owned(records, priority, &[])
}

fn resolve_owned(mut records: Vec<Recorded>, priority: &[String], ownership: &[String]) -> Report {
    let exclusive =
        |dimension: &str| exclusive(dimension) || ownership.iter().any(|d| d == dimension);
    let rank = |source: &str| {
        priority
            .iter()
            .position(|id| id == source)
            .unwrap_or(priority.len())
    };
    records.sort_by(|a, b| {
        rank(&a.source)
            .cmp(&rank(&b.source))
            .then(a.source.cmp(&b.source))
            .then(a.proposal.id.cmp(&b.proposal.id))
    });
    let mut report = Report {
        proposals: records,
        ..Report::default()
    };
    let mut winners: BTreeMap<String, String> = BTreeMap::new();
    let inactive: Vec<_> = (0..report.proposals.len())
        .map(|index| withdrawn(&report.proposals, index))
        .collect();
    for (index, record) in report.proposals.iter().enumerate() {
        let (mut status, mut by, mut reason) = ("effective", None, "source_priority");
        if inactive[index] {
            status = "withdrawn";
            by = Some(record.source.clone());
            reason = "own_withdrawal";
        } else if let Operation::Add { tag } = &record.proposal.operation {
            if let Some(exclude) = report.proposals.iter().enumerate().find(|(j, other)| {
                !inactive[*j] && rank(&other.source) < rank(&record.source)
                    && matches!(&other.proposal.operation, Operation::Exclude { selector } if selector.matches(&record.source, tag))
            }).map(|(_, r)| r) {
                status = "excluded"; by = Some(exclude.source.clone()); reason = "higher_priority_exclusion";
            } else if let Some(source) = winners.get(&tag.dimension).filter(|_| exclusive(&tag.dimension)) {
                status = "superseded"; by = Some(source.clone()); reason = "exclusive_ownership";
            } else if let Some(winner) = report.effective.iter().find(|e| e.tag.dimension == tag.dimension && e.tag.id == tag.id && (!exclusive(&tag.dimension) || e.tag.path == tag.path)) {
                status = "superseded"; by = Some(winner.source.clone()); reason = "same_tag";
            } else {
                winners.insert(tag.dimension.clone(), record.source.clone());
                report.effective.push(Effective { source: record.source.clone(), tag: tag.clone() });
            }
        } else {
            status = "recorded";
            reason = "fixed_batch_operation";
        }
        report.decisions.push(Decision {
            source: record.source.clone(),
            proposal_id: record.proposal.id.clone(),
            status: status.into(),
            by_source: by,
            reason: reason.into(),
        });
    }
    report
}

fn resolve_process(
    process: &Process,
    records: Vec<Recorded>,
    priority: &[String],
    ownership: &[String],
) -> Report {
    if !records.iter().any(|record| {
        matches!(&record.proposal.operation, Operation::Add { tag } if tag.dimension == "version")
    }) {
        return resolve_owned(records, priority, ownership);
    }
    let report = resolve_owned(records.clone(), priority, ownership);
    let name = report
        .effective
        .iter()
        .find(|e| e.tag.dimension == "name")
        .map(|e| e.tag.label.to_lowercase());
    let executable = process
        .executable
        .as_deref()
        .and_then(|path| path.file_name())
        .map(|file| file.to_string_lossy().to_lowercase());
    let runtimes: Vec<_> = report
        .effective
        .iter()
        .filter(|e| e.tag.dimension == "runtime")
        .filter_map(|e| e.tag.id.strip_prefix("runtime:"))
        .map(str::to_lowercase)
        .collect();
    let consistent = |subject: &str| {
        let subject = subject.to_lowercase();
        [
            Some(process.name.to_lowercase()),
            executable.clone(),
            name.clone(),
        ]
        .into_iter()
        .flatten()
        .chain(runtimes.iter().cloned())
        .any(|candidate| candidate == subject)
    };
    let mismatched: Vec<_> = (0..records.len())
        .filter(|index| {
            !withdrawn(&records, *index)
                && matches!(&records[*index].proposal.operation, Operation::Add { tag }
                    if tag.dimension == "version" && !consistent(tag.subject.as_deref().unwrap_or_default()))
        })
        .collect();
    if mismatched.is_empty() {
        return report;
    }
    let (rejected, kept): (Vec<_>, Vec<_>) = records
        .into_iter()
        .enumerate()
        .partition(|(index, _)| mismatched.contains(index));
    let mut report = resolve_owned(
        kept.into_iter().map(|(_, r)| r).collect(),
        priority,
        ownership,
    );
    for (_, record) in rejected {
        report.decisions.push(Decision {
            source: record.source.clone(),
            proposal_id: record.proposal.id.clone(),
            status: "rejected".into(),
            by_source: None,
            reason: "version_subject_mismatch".into(),
        });
        report.proposals.push(record);
    }
    report
}

pub(crate) fn classify(
    providers: &[&dyn Extension],
    context: &Context<'_>,
    facts: &crate::extensions::host::Facts<'_>,
) -> (BTreeMap<u32, Report>, Vec<Diagnostic>) {
    let registry = super::placement::Registry::new(providers, context);
    let ownership: Vec<_> = registry.ownership.keys().cloned().collect();
    let processes = facts.processes;
    let mut batches: BTreeMap<u32, Vec<Recorded>> = BTreeMap::new();
    let mut diagnostics = Vec::new();
    let mut rejected: BTreeMap<u32, Vec<Recorded>> = BTreeMap::new();
    let mut results = super::dispatch_many(providers, context, Request::Classify(facts));
    let priority = context.settings.source_order(
        crate::core::classifiers()
            .iter()
            .map(|p| p.descriptor().id)
            .chain(super::BUILTINS.iter().map(|p| p.descriptor().id))
            .chain(results.iter().filter_map(|(id, _)| id.clone())),
    );
    results.sort_by_key(|(source, _)| {
        priority
            .iter()
            .position(|id| Some(id) == source.as_ref())
            .unwrap_or(priority.len())
    });
    for (source, result) in results {
        let started = Instant::now();
        let source = source.unwrap_or_else(|| "unidentified".into());
        let result = result.and_then(|response| {
            let super::Contribution::Classify { proposals } = response.contribution else {
                bail!("invalid_classification");
            };
            let mut records = Vec::new();
            for (index, proposal) in proposals.into_iter().enumerate() {
                let is_role = matches!(proposal.evidence, ClassificationEvidence::Role { .. });
                let operation = match proposal.evidence {
                    ClassificationEvidence::Tags { proposal: inner } => {
                        if inner.identity != proposal.identity {
                            bail!("tag_identity_mismatch");
                        }
                        let mut proposal = inner;
                        if let Operation::Add { tag } = &mut proposal.operation {
                            registry.decorate(providers, context, facts, tag);
                        }
                        records.push(Recorded {
                            source: source.clone(),

                            proposal,
                        });
                        continue;
                    }
                    ClassificationEvidence::AppBundle { path } => {
                        let process = processes
                            .iter()
                            .find(|p| ProcessIdentity::observed(p) == Some(proposal.identity))
                            .ok_or_else(|| anyhow::anyhow!("invalid_identity"))?;
                        if process
                            .executable
                            .as_deref()
                            .and_then(crate::extensions::host::bundles::app_bundle)
                            .as_ref()
                            != Some(&path)
                        {
                            bail!("invalid_app_bundle");
                        }
                        Operation::Add {
                            tag: crate::extensions::host::proposals::app_tag(path),
                        }
                    }
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
                    } => {
                        validate_observed_path(
                            &key,
                            &label,
                            &path,
                            &basis,
                            processes,
                            proposal.identity,
                        )?;
                        let dimension = if is_role { "role" } else { "project" };
                        Operation::Add {
                            tag: Tag {
                                dimension: dimension.into(),
                                id: format!(
                                    "extensions.{source}.{key}.{}",
                                    path_id(dimension, &path)
                                        .split(':')
                                        .nth(1)
                                        .unwrap_or("unknown")
                                ),
                                label,
                                path: Some(path),
                                basis,
                                subject: None,
                                value: None,
                                projection: None,
                                supplementary_to: None,
                                evidence_chain: vec![],
                                host_resource: None,
                                assignment: None,
                            },
                        }
                    }
                };
                records.push(Recorded {
                    source: source.clone(),
                    proposal: Proposal {
                        id: format!("legacy-{index}"),
                        identity: proposal.identity,
                        operation,
                    },
                });
            }
            let contains_sensitive = |records: &[Recorded]| -> Result<bool> {
                let mut serialized = serde_json::to_string(records)?;
                for record in records {
                    if let Operation::Add { tag } = &record.proposal.operation {
                        serialized.push_str(&serde_json::to_string(&tag.projection)?);
                    }
                }
                Ok(context.settings.contains_sensitive(&serialized))
            };
            if contains_sensitive(&records)? {
                bail!("sensitive_tag_payload");
            }
            for record in &mut records {
                if let Operation::Add { tag } = &mut record.proposal.operation
                    && (exclusive(&tag.dimension) || ownership.contains(&tag.dimension))
                    && let Some(path) = &mut tag.path
                    && let Ok(canonical) = path.canonicalize()
                {
                    *path = canonical;
                }
            }
            if contains_sensitive(&records)? {
                bail!("sensitive_tag_payload");
            }
            for record in &mut records {
                if let Operation::Add { tag } = &mut record.proposal.operation {
                    registry.decorate(providers, context, facts, tag);
                }
            }
            let proposals: Vec<_> = records.iter().map(|r| r.proposal.clone()).collect();
            validate(&proposals, processes, &source, None)?;
            if proposals.iter().any(|p| {
                matches!(&p.operation, Operation::Add { tag }
                if tag.supplementary_to.is_some() && ownership.contains(&tag.dimension))
            }) {
                bail!("supplementary_ownership_conflict");
            }
            let mut by_pid: BTreeMap<u32, Vec<Recorded>> = BTreeMap::new();
            for record in records {
                by_pid
                    .entry(record.proposal.identity.pid)
                    .or_default()
                    .push(record);
            }
            if by_pid
                .values()
                .any(|records| contradictory(records, &ownership))
            {
                for (pid, records) in by_pid {
                    rejected.entry(pid).or_default().extend(records);
                }
                bail!("contradictory_tag_batch");
            }
            for (pid, records) in &by_pid {
                if batches.get(pid).map_or(0, Vec::len) + records.len() > 4096 {
                    bail!("tag_round_limit");
                }
                let mut dimensions: BTreeMap<&str, usize> = BTreeMap::new();
                for record in batches.get(pid).into_iter().flatten().chain(records) {
                    if let Operation::Add { tag } = &record.proposal.operation {
                        let count = dimensions.entry(&tag.dimension).or_default();
                        *count += 1;
                        if *count > MAX_PER_PROCESS {
                            bail!("tag_dimension_limit");
                        }
                    }
                }
            }
            context.check()?;
            Ok(by_pid)
        });
        let result = context.finish_phase(Some(&source), "classify", started.elapsed(), result);
        match result {
            Ok(records) => {
                for (pid, records) in records {
                    batches.entry(pid).or_default().extend(records);
                }
            }
            Err(_) if context.settings.is_disabled(&source) => {}
            Err(_) => diagnostics.push(super::invalid(&source, "classify")),
        }
    }
    for record in context
        .report
        .lock()
        .expect("report lock")
        .iter()
        .filter(|r| r.phase == "classify" && r.result == "skipped")
    {
        if context.settings.is_disabled(&record.provider) {
            continue;
        }
        let issue = super::invalid(&record.provider, "classify");
        if !diagnostics.iter().any(|d| d.operation == issue.operation) {
            diagnostics.push(issue);
        }
    }
    let mut reports = BTreeMap::new();
    for process in processes {
        let mut report = resolve_process(
            process,
            batches.remove(&process.pid).unwrap_or_default(),
            &priority,
            &ownership,
        );
        let primary = registry.selected(&report, context.settings).map(|entry| {
            let dimension = entry
                .tag
                .assignment
                .as_ref()
                .and_then(|a| a.group_key.as_deref())
                .and_then(|key| key.split_once('\0').map(|(dimension, _)| dimension))
                .unwrap_or(&entry.tag.dimension);
            (dimension.to_owned(), entry.tag.path.clone())
        });
        let redundant: Vec<_> = report
            .effective
            .iter()
            .filter(|entry| {
                primary.as_ref().is_some_and(|(dimension, path)| {
                    entry.tag.supplementary_to.as_ref() == Some(dimension)
                        && entry
                            .tag
                            .path
                            .as_ref()
                            .map(|p| p.canonicalize().unwrap_or_else(|_| p.clone()))
                            == path
                                .as_ref()
                                .map(|p| p.canonicalize().unwrap_or_else(|_| p.clone()))
                })
            })
            .map(|entry| (entry.source.clone(), entry.tag.id.clone()))
            .collect();
        report
            .effective
            .retain(|entry| !redundant.contains(&(entry.source.clone(), entry.tag.id.clone())));
        let redundant_proposals: Vec<_> = report
            .proposals
            .iter()
            .filter(|record| {
                matches!(&record.proposal.operation, Operation::Add { tag }
                if redundant.contains(&(record.source.clone(), tag.id.clone())))
            })
            .map(|r| (r.source.clone(), r.proposal.id.clone()))
            .collect();
        report
            .proposals
            .retain(|r| !redundant_proposals.contains(&(r.source.clone(), r.proposal.id.clone())));
        report
            .decisions
            .retain(|d| !redundant_proposals.contains(&(d.source.clone(), d.proposal_id.clone())));
        for record in rejected.remove(&process.pid).unwrap_or_default() {
            report.decisions.push(Decision {
                source: record.source.clone(),
                proposal_id: record.proposal.id.clone(),
                status: "rejected".into(),
                by_source: Some(record.source.clone()),
                reason: "contradictory_source_batch".into(),
            });
            report.proposals.push(record);
        }
        report.diagnostics = diagnostics.clone();
        if process.started_unix_us.is_none() {
            report.diagnostics.push(Diagnostic::new(
                "identity_unverified",
                "tags",
                "Process identity unavailable; tags not observed",
            ));
        }
        reports.insert(process.pid, report);
    }
    (reports, diagnostics)
}

fn validate_observed_path(
    key: &str,
    label: &str,
    path: &std::path::Path,
    basis: &str,
    processes: &[Process],
    identity: ProcessIdentity,
) -> Result<()> {
    let process = processes
        .iter()
        .find(|p| ProcessIdentity::observed(p) == Some(identity))
        .ok_or_else(|| anyhow::anyhow!("invalid_identity"))?;
    if !super::details::valid_custom(process, key, label, path, basis) {
        bail!("invalid_legacy_tag");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::{Capability, ClassificationProposal, Contribution, Descriptor};
    fn process() -> Process {
        Process {
            pid: 42,
            started_unix_us: Some(123),
            cwd: Some("/tmp".into()),
            executable: Some("/tmp/tool".into()),
            ..Default::default()
        }
    }
    fn add(id: &str, dimension: &str, tag_id: &str) -> Proposal {
        Proposal {
            id: id.into(),
            identity: ProcessIdentity::observed(&process()).unwrap(),
            operation: Operation::Add {
                tag: Tag {
                    dimension: dimension.into(),
                    id: tag_id.into(),
                    label: id.into(),
                    path: None,
                    basis: "observed".into(),
                    subject: None,
                    value: None,
                    projection: None,
                    supplementary_to: None,
                    evidence_chain: vec![],
                    host_resource: None,
                    assignment: None,
                },
            },
        }
    }
    fn exclude(id: &str, dimension: &str, tag_id: Option<&str>, source: &str) -> Proposal {
        Proposal {
            id: id.into(),
            identity: ProcessIdentity::observed(&process()).unwrap(),
            operation: Operation::Exclude {
                selector: Selector {
                    allowed_sources: vec![],
                    dimension: dimension.into(),
                    tag_id: tag_id.map(Into::into),
                    source: Some(source.into()),
                },
            },
        }
    }
    fn record(source: &str, proposal: Proposal) -> Recorded {
        Recorded {
            source: source.into(),

            proposal,
        }
    }
    #[test]
    fn priority_reverses_exclusion_and_keeps_unrelated_contributions() {
        let records = vec![
            record("a", add("test", "role", "role:test")),
            record("b", exclude("exclude-test", "role", Some("role:test"), "a")),
            record("a", add("daemon", "role", "role:daemon")),
        ];
        for (order, count, status) in [(["a", "b"], 2, "effective"), (["b", "a"], 1, "excluded")] {
            let report = resolve(records.clone(), &order.map(String::from));
            assert_eq!(report.effective.len(), count);
            assert_eq!(
                report
                    .decisions
                    .iter()
                    .find(|d| d.proposal_id == "test")
                    .unwrap()
                    .status,
                status
            );
            let mut reversed = records.clone();
            reversed.reverse();
            assert_eq!(
                serde_json::to_value(report).unwrap(),
                serde_json::to_value(resolve(reversed, &order.map(String::from))).unwrap()
            );
        }
    }
    #[test]
    fn source_specific_exclusion_and_namespaces_do_not_erase_other_adds() {
        let report = resolve(
            vec![
                record("b", exclude("exclude", "role", Some("role:test"), "a")),
                record("a", add("one", "role", "role:test")),
                record("c", add("two", "role", "role:test")),
                record("a", add("three", "role", "extensions.a.test")),
                record("c", add("four", "role", "extensions.c.test")),
            ],
            &["b".into(), "a".into(), "c".into()],
        );
        assert_eq!(report.effective.len(), 3);
        assert_eq!(
            report
                .effective
                .iter()
                .find(|e| e.tag.id == "role:test")
                .unwrap()
                .source,
            "c"
        );
    }
    #[test]
    fn withdrawal_is_local_and_mutual_exclusions_are_fixed_opinions() {
        let mut records = vec![
            record("a", add("a", "role", "role:test")),
            record("b", add("b", "role", "role:test")),
            record("a", exclude("ea", "role", None, "b")),
            record("b", exclude("eb", "role", None, "a")),
        ];
        assert_eq!(
            resolve(records.clone(), &["a".into(), "b".into()]).effective[0].source,
            "a"
        );
        records.push(record(
            "a",
            Proposal {
                id: "withdraw".into(),
                identity: ProcessIdentity::observed(&process()).unwrap(),
                operation: Operation::Withdraw {
                    proposal_id: "ea".into(),
                },
            },
        ));
        let report = resolve(records, &["a".into(), "b".into()]);
        assert_eq!(report.effective.len(), 1);
        assert!(
            report
                .decisions
                .iter()
                .any(|d| d.proposal_id == "ea" && d.status == "withdrawn")
        );
    }
    #[test]
    fn declarations_identity_and_withdrawal_targets_are_validated() {
        let mut proposal = add("test", "role", "role:test");
        let contract = Contract {
            produces: vec!["role".into()],
            overrides: vec![],
        };
        assert!(
            validate(
                std::slice::from_ref(&proposal),
                &[process()],
                "a",
                Some(&contract)
            )
            .is_ok()
        );
        proposal.identity.started_unix_us += 1;
        assert!(validate(&[proposal], &[process()], "a", Some(&contract)).is_err());
        assert!(
            validate(
                &[exclude("e", "role", Some("role:test"), "b")],
                &[process()],
                "a",
                Some(&contract)
            )
            .is_err()
        );
        let withdrawal = Proposal {
            id: "w".into(),
            identity: ProcessIdentity::observed(&process()).unwrap(),
            operation: Operation::Withdraw {
                proposal_id: "missing".into(),
            },
        };
        assert!(validate(&[withdrawal], &[process()], "a", None).is_err());
        let encoded = serde_json::to_value(add("test", "role", "role:test")).unwrap();
        let decoded: Proposal = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded.id, "test");
    }
    struct Provider(&'static str, Vec<Proposal>);
    impl Extension for Provider {
        fn descriptor(&self) -> Descriptor {
            Descriptor {
                id: self.0.into(),
                capabilities: vec![Capability::Classify],
                enabled: true,
                availability: "unknown",
            }
        }
        fn execute(&self, _: &Context<'_>, _: Request<'_>) -> Result<Contribution> {
            Ok(Contribution::Classify {
                proposals: self
                    .1
                    .iter()
                    .map(|p| ClassificationProposal {
                        identity: p.identity,
                        evidence: ClassificationEvidence::Tags {
                            proposal: p.clone(),
                        },
                    })
                    .collect(),
            })
        }
    }
    struct Off(Provider);
    impl Extension for Off {
        fn descriptor(&self) -> Descriptor {
            Descriptor {
                enabled: false,
                ..self.0.descriptor()
            }
        }
        fn execute(&self, context: &Context<'_>, request: Request<'_>) -> Result<Contribution> {
            self.0.execute(context, request)
        }
    }
    #[test]
    fn contradictory_source_isolated_and_grouping_changes_with_priority() {
        let project = |id: &str| {
            let mut p = add(id, "project", &format!("extensions.a.{id}"));
            if let Operation::Add { tag } = &mut p.operation {
                tag.path = Some("/tmp".into());
            }
            p
        };
        let a = Provider("a", vec![project("first")]);
        let b = Provider(
            "b",
            vec![exclude("exclude", "project", None, "a"), {
                let mut p = project("replacement");
                if let Operation::Add { tag } = &mut p.operation {
                    tag.id = "extensions.b.replacement".into();
                }
                p
            }],
        );
        for (order, label) in [(["a", "b"], "first"), (["b", "a"], "replacement")] {
            let config = crate::settings::Config {
                source_priority: order.map(String::from).into(),
                ..Default::default()
            };
            let settings = crate::extensions::settings_registry()
                .unwrap()
                .resolve(&config, &BTreeMap::new())
                .unwrap();
            let cancel = crate::snapshot::Cancellation::default();
            let root = crate::tree::build_with_providers(
                vec![process()],
                &Context::new(&settings, &cancel),
                &[&a, &b],
            )
            .unwrap();
            assert_eq!(root.children[0].children[0].name, label);
            assert_eq!(root.totals.processes, 1);
        }
        let bad = Provider("a", vec![project("first"), project("second")]);
        let good = Provider("b", vec![add("daemon", "role", "role:daemon")]);
        let settings = crate::extensions::default_settings().unwrap();
        let cancel = crate::snapshot::Cancellation::default();
        let (reports, diagnostics) = classify(
            &[&bad, &good],
            &Context::new(&settings, &cancel),
            &crate::extensions::host::Facts::new(&[process()]),
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(reports[&42].effective.len(), 1);
        assert_eq!(reports[&42].effective[0].source, "b");
    }
    #[test]
    fn ownership_roots_coalesce_children_without_merging_names_or_unrelated_paths() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        for root in [&first, &second] {
            std::fs::create_dir_all(root.join("src")).unwrap();
            std::fs::create_dir_all(root.join("tests")).unwrap();
        }
        let mut processes = Vec::new();
        let mut proposals = Vec::new();
        for (index, root, child) in [
            (0, &first, "src"),
            (1, &first, "tests"),
            (2, &second, "src"),
        ] {
            let mut p = process();
            p.pid += index;
            p.cwd = Some(root.join(child));
            let mut proposal = add(
                &format!("root-{index}"),
                "project",
                "extensions.a.workspace",
            );
            proposal.identity = ProcessIdentity::observed(&p).unwrap();
            if let Operation::Add { tag } = &mut proposal.operation {
                tag.label = "Same name".into();
                tag.path = Some(root.clone());
                tag.basis = "cwd".into();
            }
            processes.push(p);
            proposals.push(proposal);
        }
        let contract = Contract {
            produces: vec!["project".into()],
            overrides: vec![],
        };
        validate(&proposals, &processes, "a", Some(&contract)).unwrap();
        for bad_root in [
            second.clone(),
            temp.path().join("missing"),
            PathBuf::from("relative"),
        ] {
            let mut bad = proposals[0].clone();
            if let Operation::Add { tag } = &mut bad.operation {
                tag.path = Some(bad_root);
            }
            assert!(validate(&[bad], &processes, "a", Some(&contract)).is_err());
        }
        let provider = Provider("a", proposals);
        let settings = crate::extensions::default_settings().unwrap();
        let cancel = crate::snapshot::Cancellation::default();
        let root = crate::tree::build_with_providers(
            processes,
            &Context::new(&settings, &cancel),
            &[&provider],
        )
        .unwrap();
        let extensions = crate::tree::select(&root, "group:projects").unwrap();
        assert_eq!(extensions.children.len(), 2);
        assert!(extensions.children.iter().any(|n| n.totals.processes == 2));
        assert_eq!(root.totals.processes, 3);
        temp.close().unwrap();
    }

    #[test]
    fn app_roots_require_observed_bundle_lineage_for_all_helpers() {
        let temp = tempfile::tempdir().unwrap();
        let bundle = temp.path().join("Demo.app");
        let mut processes = Vec::new();
        let mut proposals = Vec::new();
        for (index, relative) in [(0, "Contents/MacOS/main"), (1, "Contents/Helpers/worker")] {
            let executable = bundle.join(relative);
            std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
            std::fs::write(&executable, "fixture").unwrap();
            let mut p = process();
            p.pid += index;
            p.executable = Some(executable);
            let mut proposal = add(&format!("helper-{index}"), "app", "extensions.a.app");
            proposal.identity = ProcessIdentity::observed(&p).unwrap();
            if let Operation::Add { tag } = &mut proposal.operation {
                tag.path = Some(bundle.clone());
                tag.basis = "executable".into();
            }
            processes.push(p);
            proposals.push(proposal);
        }
        let contract = Contract {
            produces: vec!["app".into()],
            overrides: vec![],
        };
        validate(&proposals, &processes, "a", Some(&contract)).unwrap();
        let mut unrelated = proposals[0].clone();
        if let Operation::Add { tag } = &mut unrelated.operation {
            tag.path = Some(temp.path().into());
        }
        assert!(validate(&[unrelated], &processes, "a", Some(&contract)).is_err());
        temp.close().unwrap();
    }

    #[test]
    fn removing_builtin_app_and_bin_changes_primary_without_resurrecting_raw_paths() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("go.mod"), "module fixture").unwrap();
        let mut p = process();
        p.cwd = Some(temp.path().into());
        p.executable = Some("/Applications/Demo.app/Contents/MacOS/main".into());
        let exclude_app = Provider(
            "override",
            vec![exclude("remove-app", "app", None, "app_bundle")],
        );
        let settings = crate::extensions::settings_registry()
            .unwrap()
            .resolve(
                &crate::settings::Config {
                    source_priority: vec!["override".into()],
                    ..Default::default()
                },
                &BTreeMap::new(),
            )
            .unwrap();
        let cancel = crate::snapshot::Cancellation::default();
        let root = crate::tree::build_with_providers(
            vec![p],
            &Context::new(&settings, &cancel),
            &[&crate::core::Core::AppBundle, &exclude_app],
        )
        .unwrap();
        assert!(crate::tree::select(&root, "group:applications").is_err());
        assert_eq!(
            crate::tree::select(&root, "group:projects")
                .unwrap()
                .totals
                .processes,
            1
        );
        let mut system = process();
        system.executable = Some("/usr/libexec/fixture".into());
        let exclude_bin = Provider("override", vec![exclude("remove-bin", "bin", None, "bin")]);
        let root = crate::tree::build_with_providers(
            vec![system],
            &Context::new(&settings, &cancel),
            &[&exclude_bin],
        )
        .unwrap();
        assert!(crate::tree::select(&root, "group:system").is_err());
        assert_eq!(root.totals.processes, 1);
        assert!(crate::tree::select(&root, "pid:42").is_ok());
        let unknown = crate::tree::select(&root, "group:unclassified").unwrap();
        assert!(
            unknown
                .children
                .iter()
                .all(|n| !matches!(n.grouping, Some(crate::model::Grouping::Bin)))
        );
        temp.close().unwrap();
    }
    fn typed(id: &str, dimension: &str, subject: Option<&str>, value: Option<&str>) -> Proposal {
        let mut proposal = add(id, dimension, &format!("{dimension}:{id}"));
        if let Operation::Add { tag } = &mut proposal.operation {
            tag.subject = subject.map(Into::into);
            tag.value = value.map(Into::into);
            tag.basis = "executable".into();
        }
        proposal
    }

    #[test]
    fn version_and_name_tags_require_typed_fields_without_paths() {
        let contract = Contract {
            produces: vec![
                "version".into(),
                "name".into(),
                "work".into(),
                "role".into(),
            ],
            overrides: vec![],
        };
        let check =
            |proposal: Proposal| validate(&[proposal], &[process()], "a", Some(&contract)).is_ok();
        assert!(check(typed("v", "version", Some("tool"), Some("1.2.3"))));
        assert!(check(typed("n", "name", None, None)));
        assert!(check(typed("w", "work", None, None)));
        assert!(!check(typed("w", "work", Some("tool"), None)));
        assert!(!check(typed("v", "version", None, Some("1.2.3"))));
        assert!(!check(typed("v", "version", Some("tool"), None)));
        assert!(!check(typed("v", "version", Some(""), Some("1.2.3"))));
        assert!(!check(typed("r", "role", Some("tool"), None)));
        assert!(!check(typed("n", "name", None, Some("1.2.3"))));
        for dimension in ["version", "name", "work"] {
            let mut with_path = if dimension == "version" {
                typed("p", dimension, Some("tool"), Some("1"))
            } else {
                typed("p", dimension, None, None)
            };
            if let Operation::Add { tag } = &mut with_path.operation {
                tag.path = Some("/tmp/tool".into());
            }
            assert!(!check(with_path), "{dimension}");
        }
        let encoded = serde_json::to_value(typed("v", "version", Some("tool"), Some("1"))).unwrap();
        assert_eq!(encoded["change"]["tag"]["subject"], "tool");
        let role = serde_json::to_value(add("r", "role", "role:r")).unwrap();
        assert!(role["change"]["tag"].get("subject").is_none());
    }

    #[test]
    fn version_subject_is_checked_before_priority_and_disabled_sources_vanish() {
        let version =
            |id: &str, subject: &str, value: &str| typed(id, "version", Some(subject), Some(value));
        let name = |id: &str, label: &str| {
            let mut p = typed(id, "name", None, None);
            if let Operation::Add { tag } = &mut p.operation {
                tag.label = label.into();
            }
            p
        };
        let run = |providers: &[&dyn Extension]| {
            let settings = crate::extensions::settings_registry()
                .unwrap()
                .resolve(
                    &crate::settings::Config {
                        source_priority: vec!["a".into(), "b".into(), "c".into()],
                        ..Default::default()
                    },
                    &BTreeMap::new(),
                )
                .unwrap();
            let cancel = crate::snapshot::Cancellation::default();
            let (reports, _) = classify(
                providers,
                &Context::new(&settings, &cancel),
                &crate::extensions::host::Facts::new(&[process()]),
            );
            reports[&42].clone()
        };
        let a = Provider("a", vec![version("other", "other", "9.9.9")]);
        let b = Provider("b", vec![version("tool", "TOOL", "1.2.3")]);
        let c = Provider(
            "c",
            vec![
                name("label", "claude"),
                version("claude", "Claude", "2.1.316"),
            ],
        );
        let report = run(&[&a, &b]);
        let effective: Vec<_> = report
            .effective
            .iter()
            .filter(|e| e.tag.dimension == "version")
            .collect();
        assert_eq!(effective.len(), 1);
        assert_eq!(
            (
                effective[0].source.as_str(),
                effective[0].tag.value.as_deref()
            ),
            ("b", Some("1.2.3"))
        );
        let decision = |report: &Report, id: &str| {
            report
                .decisions
                .iter()
                .find(|d| d.proposal_id == id)
                .unwrap()
                .clone()
        };
        assert_eq!(decision(&report, "other").status, "rejected");
        assert_eq!(
            decision(&report, "other").reason,
            "version_subject_mismatch"
        );
        assert!(report.proposals.iter().any(|r| r.proposal.id == "other"));
        let report = run(&[&b, &c]);
        assert_eq!(decision(&report, "tool").status, "effective");
        assert_eq!(decision(&report, "claude").status, "superseded");
        assert_eq!(decision(&report, "claude").reason, "exclusive_ownership");
        assert_eq!(
            report
                .effective
                .iter()
                .find(|e| e.tag.dimension == "name")
                .unwrap()
                .tag
                .label,
            "claude"
        );
        let report = run(&[&Off(b), &c]);
        let winner = report
            .effective
            .iter()
            .find(|e| e.tag.dimension == "version")
            .unwrap();
        assert_eq!(
            (winner.source.as_str(), winner.tag.value.as_deref()),
            ("c", Some("2.1.316"))
        );
        assert!(report.proposals.iter().all(|r| r.source != "b"));
        let b = Provider("b", vec![version("tool", "TOOL", "1.2.3")]);
        let twice = Provider(
            "a",
            vec![version("one", "tool", "1"), version("two", "tool", "2")],
        );
        let report = run(&[&twice, &b]);
        assert_eq!(
            report
                .effective
                .iter()
                .find(|e| e.tag.dimension == "version")
                .unwrap()
                .source,
            "b"
        );
    }

    #[test]
    fn work_labels_take_display_precedence_over_name_tags() {
        let named = |id: &str, dimension: &str, label: &str| {
            let mut p = typed(id, dimension, None, None);
            if let Operation::Add { tag } = &mut p.operation {
                tag.label = label.into();
            }
            p
        };
        let mut os = process();
        os.name = "node".into();
        let settings = crate::extensions::default_settings().unwrap();
        let cancel = crate::snapshot::Cancellation::default();
        for (proposals, name, reason) in [
            (
                vec![
                    named("n", "name", "tool"),
                    named("w", "work", "firecrawl-mcp"),
                ],
                "firecrawl-mcp",
                Some(crate::model::NameReason::Work),
            ),
            (
                vec![named("n", "name", "tool")],
                "tool",
                Some(crate::model::NameReason::Name),
            ),
            (vec![], "node", None),
        ] {
            let provider = Provider("a", proposals);
            let tree = crate::tree::build_with_providers(
                vec![os.clone()],
                &Context::new(&settings, &cancel),
                &[&provider],
            )
            .unwrap();
            let process = crate::tree::select(&tree, "pid:42")
                .unwrap()
                .process
                .unwrap();
            assert_eq!(process.name, name);
            assert_eq!(process.name_source.map(|s| s.reason), reason);
            assert_eq!(process.original_name.as_deref(), reason.map(|_| "node"));
        }
    }

    #[test]
    fn version_subject_may_match_an_effective_runtime_tag() {
        let runtime = |id: &str| {
            let mut p = add(id, "runtime", &format!("runtime:{id}"));
            if let Operation::Add { tag } = &mut p.operation {
                tag.label = id.into();
            }
            p
        };
        let version = typed("v", "version", Some("Python"), Some("3.12.1"));
        let settings = crate::extensions::default_settings().unwrap();
        let cancel = crate::snapshot::Cancellation::default();
        for (proposals, accepted) in [
            (vec![runtime("python"), version.clone()], true),
            (vec![runtime("node"), version.clone()], false),
            (vec![version], false),
        ] {
            let provider = Provider("a", proposals);
            let (reports, _) = classify(
                &[&provider],
                &Context::new(&settings, &cancel),
                &crate::extensions::host::Facts::new(&[process()]),
            );
            assert_eq!(
                reports[&42]
                    .effective
                    .iter()
                    .any(|e| e.tag.dimension == "version"),
                accepted
            );
        }
    }

    #[test]
    fn sensitive_settings_cannot_be_echoed_into_tag_trace() {
        let secret = "fixture-\"value";
        let mut definition: crate::settings::Definition = serde_json::from_value(serde_json::json!({"key":"extensions.a.token","type":"string","default":secret,"description":"fixture","sensitive":true})).unwrap();
        definition.provider = "a".into();
        let mut registry = crate::extensions::settings_registry().unwrap();
        registry
            .register_extension("a", vec![definition], vec![])
            .unwrap();
        let settings = registry
            .resolve(&crate::settings::Config::default(), &BTreeMap::new())
            .unwrap();
        let mut leaked = add("leak", "role", "role:test");
        if let Operation::Add { tag } = &mut leaked.operation {
            tag.label = secret.into();
        }
        let mut metadata_leak = add("metadata-leak", "role", "role:test");
        if let Operation::Add { tag } = &mut metadata_leak.operation {
            tag.projection=Some(serde_json::from_value(serde_json::json!({"grouping":"unknown","parents":[{"reference":"group:extensions.a.parent","name":secret,"grouping":"unknown","evidence":[],"attribution":null,"diagnostics":[],"collapse_single":false,"child_name":null}]})).unwrap());
        }
        for leaked in [leaked, metadata_leak] {
            let bad = Provider("a", vec![leaked]);
            let good = Provider("b", vec![add("daemon", "role", "role:daemon")]);
            let cancel = crate::snapshot::Cancellation::default();
            let (reports, diagnostics) = classify(
                &[&bad, &good],
                &Context::new(&settings, &cancel),
                &crate::extensions::host::Facts::new(&[process()]),
            );
            assert_eq!(diagnostics.len(), 1);
            assert_eq!(reports[&42].proposals.len(), 1);
            assert_eq!(reports[&42].proposals[0].source, "b");
        }
    }
    #[test]
    fn review_common_ownership_identity_coalesces_builtin_and_module_winners() {
        let temp = tempfile::tempdir().unwrap();
        for dimension in ["project", "app", "bin"] {
            let root = temp.path().join(if dimension == "app" {
                "Demo.app"
            } else {
                dimension
            });
            std::fs::create_dir_all(&root).unwrap();
            if dimension == "project" {
                std::fs::write(root.join("go.mod"), "module fixture").unwrap();
                std::fs::create_dir(root.join(".git")).unwrap();
            }
            let mut first = process();
            first.cwd = Some(root.clone());
            if dimension == "app" {
                first.executable = Some(root.join("Contents/MacOS/main"));
            }
            if dimension == "bin" {
                first.executable = Some(root.clone());
            }
            let mut second = first.clone();
            second.pid += 1;
            let mut proposal = add("override", dimension, &path_id(dimension, &root));
            if let Operation::Add { tag } = &mut proposal.operation {
                tag.path = Some(root.clone());
                tag.basis = if dimension == "project" {
                    "cwd"
                } else {
                    "executable"
                }
                .into();
            }
            let provider = Provider("override", vec![proposal]);
            let settings = crate::extensions::settings_registry()
                .unwrap()
                .resolve(
                    &crate::settings::Config {
                        source_priority: vec!["override".into()],
                        ..Default::default()
                    },
                    &BTreeMap::new(),
                )
                .unwrap();
            let cancel = crate::snapshot::Cancellation::default();
            let tree = crate::tree::build_with_providers(
                vec![first.clone(), second.clone()],
                &Context::new(&settings, &cancel),
                &[&crate::core::Core::AppBundle, &provider],
            )
            .unwrap();
            let reference = ownership_ref(dimension, &path_id(dimension, &root), &root);
            let group = crate::tree::select(&tree, &reference).unwrap();
            assert_eq!(group.totals.processes, 2, "{dimension}");
            if dimension == "project" {
                assert_eq!(
                    group.attribution.as_ref().unwrap().checkout_path.as_ref(),
                    Some(&root.canonicalize().unwrap())
                );
            }
            assert_eq!(tree.totals.processes, 2);
            let mut distinct = add("distinct", dimension, "extensions.override.distinct");
            if let Operation::Add { tag } = &mut distinct.operation {
                tag.path = Some(root);
            }
            let other = Provider("override", vec![distinct]);
            let tree = crate::tree::build_with_providers(
                vec![first, second],
                &Context::new(&settings, &cancel),
                &[&crate::core::Core::AppBundle, &other],
            )
            .unwrap();
            let a = crate::tree::select(&tree, "pid:42")
                .unwrap()
                .process
                .unwrap()
                .tag_report
                .unwrap();
            let b = crate::tree::select(&tree, "pid:43")
                .unwrap()
                .process
                .unwrap()
                .tag_report
                .unwrap();
            assert_ne!(
                a.effective
                    .iter()
                    .find(|e| e.tag.dimension == dimension)
                    .unwrap()
                    .tag
                    .id,
                b.effective
                    .iter()
                    .find(|e| e.tag.dimension == dimension)
                    .unwrap()
                    .tag
                    .id
            );
            if dimension != "bin" {
                for report in [&a, &b] {
                    let tag = &report
                        .effective
                        .iter()
                        .find(|e| e.tag.dimension == dimension)
                        .unwrap()
                        .tag;
                    let reference = ownership_ref(dimension, &tag.id, tag.path.as_ref().unwrap());
                    assert_eq!(
                        crate::tree::select(&tree, &reference)
                            .unwrap()
                            .totals
                            .processes,
                        1
                    );
                }
            }
            crate::tree::validate_references(&tree).unwrap();
        }
        temp.close().unwrap();
    }

    #[test]
    fn review_bin_roots_and_namespaces_remain_distinct_across_categories() {
        let mut processes = Vec::new();
        let mut proposals = Vec::new();
        for (index, path) in [
            "/tmp/review-a",
            "/tmp/review-a",
            "/usr/libexec/review-b",
            "/usr/libexec/review-b",
        ]
        .iter()
        .enumerate()
        {
            let mut p = process();
            p.pid += index as u32;
            p.executable = Some((*path).into());
            p.resident_bytes = Some(100);
            p.cpu_percent = Some(2.5);
            let mut proposal = add(&format!("bin-{index}"), "bin", "extensions.override.bin");
            proposal.identity = ProcessIdentity::observed(&p).unwrap();
            if let Operation::Add { tag } = &mut proposal.operation {
                tag.path = p.executable.clone();
                tag.basis = "executable".into();
            }
            processes.push(p);
            proposals.push(proposal);
        }
        let provider = Provider("override", proposals);
        let settings = crate::extensions::settings_registry()
            .unwrap()
            .resolve(
                &crate::settings::Config {
                    source_priority: vec!["override".into()],
                    ..Default::default()
                },
                &BTreeMap::new(),
            )
            .unwrap();
        let cancel = crate::snapshot::Cancellation::default();
        let tree = crate::tree::build_with_providers(
            processes,
            &Context::new(&settings, &cancel),
            &[&provider],
        )
        .unwrap();
        for path in ["/tmp/review-a", "/usr/libexec/review-b"] {
            let reference =
                ownership_ref("bin", "extensions.override.bin", std::path::Path::new(path));
            assert_eq!(
                crate::tree::select(&tree, &reference)
                    .unwrap()
                    .totals
                    .processes,
                2
            );
        }
        assert_eq!(tree.totals.processes, 4);
        assert_eq!(tree.totals.resident_bytes, 400);
        assert_eq!(tree.totals.cpu_percent, 10.0);
        assert_eq!(
            crate::tree::select(&tree, "group:system")
                .unwrap()
                .totals
                .processes,
            2
        );
        assert_eq!(
            crate::tree::select(&tree, "group:unclassified")
                .unwrap()
                .totals
                .processes,
            2
        );
        assert_ne!(
            ownership_ref(
                "bin",
                "extensions.override.bin",
                std::path::Path::new("/tmp/review-a")
            ),
            ownership_ref(
                "bin",
                "extensions.other.bin",
                std::path::Path::new("/tmp/review-a")
            )
        );
        crate::tree::validate_references(&tree).unwrap();
    }

    #[test]
    fn review_builtin_unicode_labels_are_bounded_without_losing_healthy_siblings() {
        let temp = tempfile::tempdir().unwrap();
        let long = "界".repeat(60);
        let project = temp.path().join(&long);
        let healthy = temp.path().join("healthy");
        let app = temp.path().join(format!("{long}.app"));
        let binary = temp.path().join(format!("{long}-bin"));
        for root in [&project, &healthy] {
            std::fs::create_dir_all(root).unwrap();
            std::fs::write(root.join("go.mod"), "module fixture").unwrap();
        }
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        std::fs::write(&binary, "fixture").unwrap();
        let mut processes = Vec::new();
        for (index, cwd) in [&project, &healthy, temp.path(), temp.path(), temp.path()]
            .iter()
            .enumerate()
        {
            let mut p = process();
            p.pid += index as u32;
            p.cwd = Some(cwd.to_path_buf());
            p.uid = Some(501);
            p.started_unix_us = Some(123 + index as u64);
            if index == 2 {
                p.executable = Some(app.join("Contents/MacOS/main"));
            }
            if index == 3 {
                p.executable = Some(binary.clone());
            }
            if index == 4 {
                p.executable = None;
                p.ppid = Some(44);
            }
            processes.push(p);
        }
        let tree = crate::tree::build(processes).unwrap();
        assert!(tree.diagnostics.is_empty(), "{:?}", tree.diagnostics);
        assert_eq!(
            crate::tree::select(&tree, "group:projects")
                .unwrap()
                .totals
                .processes,
            2
        );
        assert_eq!(tree.totals.processes, 5);
        for pid in 42..47 {
            let report = crate::tree::select(&tree, &format!("pid:{pid}"))
                .unwrap()
                .process
                .unwrap()
                .tag_report
                .unwrap();
            assert!(
                report
                    .effective
                    .iter()
                    .all(|e| e.tag.label.len() <= 128 && !e.tag.label.is_empty())
            );
        }
        let inferred = crate::tree::select(&tree, "pid:46")
            .unwrap()
            .process
            .unwrap()
            .tag_report
            .unwrap();
        assert!(inferred.effective.iter().any(|e| e.source == "ancestor"));
        temp.close().unwrap();
    }
    #[test]
    fn review_module_oversized_labels_still_reject_the_entire_source() {
        let mut invalid = add("bad", "role", "role:test");
        if let Operation::Add { tag } = &mut invalid.operation {
            tag.label = "界".repeat(60);
        }
        let bad = Provider(
            "bad",
            vec![invalid, add("otherwise-valid", "role", "role:worker")],
        );
        let good = Provider("good", vec![add("healthy", "role", "role:daemon")]);
        let settings = crate::extensions::default_settings().unwrap();
        let cancel = crate::snapshot::Cancellation::default();
        let (reports, diagnostics) = classify(
            &[&bad, &good],
            &Context::new(&settings, &cancel),
            &crate::extensions::host::Facts::new(&[process()]),
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(reports[&42].effective.len(), 1);
        assert_eq!(reports[&42].effective[0].source, "good");
    }
}
