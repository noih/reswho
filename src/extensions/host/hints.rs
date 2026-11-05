use crate::{
    extensions::{
        ClassificationEvidence, ClassificationProposal, ProcessIdentity,
        tags::{Hint, Operation, ProcessEvidence, Proposal, Tag},
    },
    model::{Node, Process, SummaryHint},
};
use std::{
    collections::{BTreeMap, HashSet},
    path::PathBuf,
};

pub(crate) fn chain(facts: &super::Facts<'_>, process: &Process) -> Vec<ProcessEvidence> {
    let mut current = process;
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    while let Some(identity) = ProcessIdentity::observed(current) {
        let Some(executable) = current.executable.as_ref().filter(|p| p.is_absolute()) else {
            break;
        };
        if !seen.insert(current.pid) || result.len() >= 128 {
            return Vec::new();
        }
        result.push(ProcessEvidence {
            identity,
            executable: executable.clone(),
        });
        let Some(parent) = current.ppid.and_then(|pid| facts.by_pid.get(&pid)).copied() else {
            break;
        };
        if seen.contains(&parent.pid) {
            return Vec::new();
        }
        if parent.pid <= 1
            || current.uid.is_none()
            || current.uid != parent.uid
            || parent.started_unix_us.is_none()
            || parent.started_unix_us > current.started_unix_us
        {
            break;
        }
        current = parent;
    }
    result
}

pub(crate) fn proposal(
    source: &str,
    mut hint: Hint,
    basis: &str,
    path: Option<PathBuf>,
    chain: Vec<ProcessEvidence>,
) -> Option<ClassificationProposal> {
    let identity = chain.first()?.identity;
    hint.evidence_chain = chain;
    let dimension = format!("extensions.{source}.{}", hint.kind);
    let id = super::stable_ref(&dimension, &format!("{}\0{path:?}", hint.value));
    Some(ClassificationProposal {
        identity,
        evidence: ClassificationEvidence::Tags {
            proposal: Proposal {
                id: format!("{source}-{}-{id}", hint.kind),
                identity,
                operation: Operation::Add {
                    tag: Tag {
                        dimension: dimension.clone(),
                        id: format!("{dimension}:{}", id.trim_start_matches("group:")),
                        label: super::proposals::label(&hint.value),
                        path,
                        basis: basis.into(),
                        hint: Some(Box::new(hint)),
                        subject: None,
                        value: None,
                        projection: None,
                        assignment: None,
                        supplementary_to: None,
                        evidence_chain: vec![],
                        host_resource: None,
                    },
                },
            },
        },
    })
}

pub(crate) fn value(kind: &str, value: &str) -> Hint {
    Hint {
        record: None,
        evidence_chain: vec![],
        kind: kind.into(),
        value: value.into(),
        title: None,
    }
}

pub(crate) fn summarize(node: &mut Node) -> BTreeMap<SummaryHint, u64> {
    let mut counts = BTreeMap::new();
    for child in &mut node.children {
        for (hint, count) in summarize(child) {
            *counts.entry(hint).or_default() += count;
        }
    }
    node.hints.clear();
    if let Some(process) = &node.process {
        for entry in process
            .tag_report
            .iter()
            .flat_map(|report| &report.effective)
        {
            let tag = &entry.tag;
            let Some(hint) = tag.hint.as_deref() else {
                continue;
            };
            if hint.kind == "session_unavailable" {
                continue;
            }
            let summary = SummaryHint {
                source: entry.source.clone(),
                basis: tag.basis.clone(),
                kind: hint.kind.clone(),
                value: hint.value.clone(),
                title: hint.title.clone(),
                path: tag.path.clone(),
                processes: 0,
            };
            if !node.hints.iter().any(|existing: &SummaryHint| {
                let mut key = existing.clone();
                key.processes = 0;
                key == summary
            }) {
                *counts.entry(summary.clone()).or_default() += 1;
                node.hints.push(SummaryHint {
                    processes: 1,
                    ..summary
                });
            }
        }
    } else if node.container.is_none() && node.runtime.is_none() && node.resource.is_none() {
        node.hints = counts
            .iter()
            .map(|(hint, count)| SummaryHint {
                processes: *count,
                ..hint.clone()
            })
            .collect();
    }
    counts
}

#[cfg(test)]
mod tests;
