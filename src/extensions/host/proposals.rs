use super::assignment::Assignment;
use super::stable_ref;
use crate::extensions::{
    ClassificationEvidence, ClassificationProposal, ProcessIdentity,
    tags::{Operation, Proposal, Tag},
};
use std::path::PathBuf;

pub(crate) fn label(value: &str) -> String {
    let mut label = String::new();
    for character in value.chars().filter(|c| !c.is_control()) {
        if label.len() + character.len_utf8() > 128 {
            break;
        }
        label.push(character);
    }
    if label.is_empty() {
        "Unknown".into()
    } else {
        label
    }
}

pub(crate) fn app_tag(path: PathBuf) -> Tag {
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .trim_end_matches(".bundle")
        .trim_end_matches(".app")
        .to_owned();
    Tag {
        hint: None,
        dimension: "app".into(),
        id: crate::extensions::tags::path_id("app", &path),
        label: label(&name),
        path: Some(path),
        basis: "app_bundle".into(),
        subject: None,
        value: None,
        projection: None,
        supplementary_to: None,
        evidence_chain: vec![],
        host_resource: None,
        assignment: None,
    }
}

pub(crate) fn identity_proposal(
    identity: ProcessIdentity,
    source: &str,
    basis: &str,
    dimension: &str,
    label: &str,
    subject: Option<&str>,
) -> ClassificationProposal {
    let key = format!("{}\0{label}", subject.unwrap_or_default());
    let tag = Tag {
        hint: None,
        dimension: dimension.into(),
        id: format!(
            "{dimension}:{}",
            stable_ref(dimension, &key).trim_start_matches("group:")
        ),
        label: label.into(),
        path: None,
        basis: basis.into(),
        subject: subject.map(Into::into),
        value: subject.map(|_| label.into()),
        projection: None,
        supplementary_to: None,
        evidence_chain: vec![],
        host_resource: None,
        assignment: None,
    };
    let proposal = Proposal {
        id: format!("{source}-{dimension}-{}", identity.pid),
        identity,
        operation: Operation::Add { tag },
    };
    ClassificationProposal {
        identity,
        evidence: ClassificationEvidence::Tags { proposal },
    }
}

pub(crate) fn with_id(mut proposal: ClassificationProposal, id: String) -> ClassificationProposal {
    if let ClassificationEvidence::Tags { proposal } = &mut proposal.evidence
        && let Operation::Add { tag } = &mut proposal.operation
    {
        tag.id = id;
    }
    proposal
}

pub(crate) fn ownership(
    identity: ProcessIdentity,
    source: &str,
    dimension: &str,
    id: String,
    name: &str,
    path: PathBuf,
    assigned: Assignment,
) -> ClassificationProposal {
    let tag = Tag {
        hint: None,
        dimension: dimension.into(),
        id,
        label: label(name),
        path: Some(path),
        basis: source.into(),
        subject: None,
        value: None,
        projection: assigned.projection.clone().map(Box::new),
        supplementary_to: None,
        evidence_chain: vec![],
        host_resource: None,
        assignment: Some(Box::new(assigned)),
    };
    ClassificationProposal {
        identity,
        evidence: ClassificationEvidence::Tags {
            proposal: Proposal {
                id: format!("{source}-{}", identity.pid),
                identity,
                operation: Operation::Add { tag },
            },
        },
    }
}
