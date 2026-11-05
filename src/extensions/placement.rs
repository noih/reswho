use super::{Context, Extension, host::Facts, tags::Tag};
use crate::model::{Attribution, Diagnostic, Grouping};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registration {
    #[serde(default)]
    pub categories: Vec<Category>,
    #[serde(default)]
    pub ownership: Vec<Ownership>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Category {
    pub id: String,
    pub label: String,
    #[serde(deserialize_with = "crate::model::integer::integer")]
    pub order: i32,
}
impl Category {
    pub fn new(id: &str, label: &str, order: i32) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            order,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ownership {
    pub dimension: String,
    pub category: String,
    #[serde(deserialize_with = "crate::model::integer::integer")]
    pub priority: i32,
    #[serde(default = "unknown_grouping")]
    pub grouping: Grouping,
}
impl Ownership {
    pub fn new(dimension: &str, category: &str, priority: i32) -> Self {
        Self {
            dimension: dimension.into(),
            category: category.into(),
            priority,
            grouping: unknown_grouping(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Projection {
    pub grouping: Grouping,
    #[serde(default)]
    pub parents: Vec<Parent>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Parent {
    pub reference: String,
    #[serde(default)]
    pub merge: bool,
    #[serde(default)]
    pub collision_reference: Option<String>,
    pub name: String,
    pub grouping: Grouping,
    pub evidence: Vec<String>,
    pub attribution: Option<Attribution>,
    pub diagnostics: Vec<Diagnostic>,
    pub collapse_single: bool,
    pub child_name: Option<String>,
}
fn clean(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}
fn id(value: &str) -> bool {
    clean(value, 128)
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}
impl Registration {
    pub fn validate(&self, provider: &str) -> anyhow::Result<()> {
        let mut categories = HashSet::new();
        let mut dimensions = HashSet::new();
        if self.categories.len() > 32
            || self.ownership.len() > 32
            || self.categories.iter().any(|c| {
                !id(&c.id)
                    || !clean(&c.label, 128)
                    || !categories.insert(&c.id)
                    || matches!(
                        c.id.as_str(),
                        "all" | "applications" | "system" | "unclassified" | "resources"
                    )
            })
            || self.ownership.iter().any(|o| {
                !id(&o.dimension)
                    || !id(&o.category)
                    || matches!(o.grouping, Grouping::Runtime | Grouping::Container)
                    || !dimensions.insert(&o.dimension)
                    || matches!(
                        o.dimension.as_str(),
                        "app" | "bin" | "version" | "name" | "role" | "runtime"
                    )
                    || !(self.categories.iter().any(|c| c.id == o.category)
                        || matches!(
                            o.category.as_str(),
                            "unclassified" | "applications" | "system"
                        ))
            })
        {
            anyhow::bail!("invalid_placement_registration");
        }
        if self.categories.iter().any(|c| {
            !matches!(c.id.as_str(), "projects" | "containers")
                && !c.id.starts_with(&format!("extensions.{provider}."))
        }) || self.ownership.iter().any(|o| {
            !matches!(o.dimension.as_str(), "project" | "work")
                && !o.dimension.starts_with(&format!("extensions.{provider}."))
        }) {
            anyhow::bail!("invalid_placement_namespace");
        }
        Ok(())
    }
}
pub(crate) struct Registry {
    pub categories: BTreeMap<String, Category>,
    pub ownership: BTreeMap<String, Ownership>,
}
impl Registry {
    pub fn selected<'a>(
        &self,
        report: &'a super::tags::Report,
        settings: &crate::settings::Resolved,
    ) -> Option<&'a super::tags::Effective> {
        report
            .effective
            .iter()
            .filter(|entry| self.ownership.contains_key(&entry.tag.dimension))
            .min_by_key(|entry| {
                (
                    settings
                        .ownership_priority
                        .iter()
                        .position(|id| id == &entry.tag.dimension)
                        .unwrap_or(settings.ownership_priority.len()),
                    self.ownership[&entry.tag.dimension].priority,
                    &entry.tag.dimension,
                )
            })
    }

    pub fn new(providers: &[&dyn Extension], context: &Context<'_>) -> Self {
        let core = crate::core::registration();
        let mut registry = Self {
            categories: core
                .categories
                .into_iter()
                .chain(
                    super::BUILTINS
                        .iter()
                        .flat_map(|provider| provider.registration().categories),
                )
                .map(|c| (c.id.clone(), c))
                .collect(),
            ownership: core
                .ownership
                .into_iter()
                .map(|o| (o.dimension.clone(), o))
                .collect(),
        };
        for provider in providers {
            let result = super::boundary(|| {
                let registration = provider.registration();
                if registration.categories.is_empty() && registration.ownership.is_empty() {
                    return Ok(());
                }
                let descriptor = provider.descriptor();
                if !descriptor.enabled || context.settings.is_disabled(&descriptor.id) {
                    return Ok(());
                }
                registration.validate(&descriptor.id)?;
                if context
                    .settings
                    .contains_sensitive(&serde_json::to_string(&registration)?)
                {
                    anyhow::bail!("sensitive_placement_registration");
                }
                if registration
                    .categories
                    .iter()
                    .any(|c| registry.categories.get(&c.id).is_some_and(|old| old != c))
                    || registration.ownership.iter().any(|o| {
                        registry
                            .ownership
                            .get(&o.dimension)
                            .is_some_and(|old| old != o)
                    })
                {
                    anyhow::bail!("colliding_placement_registration");
                }
                for c in registration.categories {
                    registry.categories.insert(c.id.clone(), c);
                }
                for o in registration.ownership {
                    registry.ownership.insert(o.dimension.clone(), o);
                }
                Ok(())
            });
            if result.is_err()
                && let Ok(descriptor) = super::boundary(|| Ok(provider.descriptor()))
            {
                let _ = context.finish::<()>(Some(&descriptor.id), result);
            }
        }
        registry
    }
    pub fn decorate(
        &self,
        providers: &[&dyn Extension],
        context: &Context<'_>,
        facts: &Facts<'_>,
        tag: &mut Tag,
    ) {
        for provider in providers {
            let mut candidate = tag.clone();
            let result = super::boundary(|| {
                let Some(decorate) = provider.decoration() else {
                    return Ok(());
                };
                let descriptor = provider.descriptor();
                if descriptor.enabled
                    && !context.settings.is_disabled(&descriptor.id)
                    && !context
                        .failed
                        .lock()
                        .expect("round failure lock")
                        .contains(&descriptor.id)
                {
                    decorate(facts, &mut candidate);
                }
                Ok(())
            });
            if result.is_ok() {
                *tag = candidate;
            }
        }
    }
}

fn unknown_grouping() -> Grouping {
    Grouping::Unknown
}

pub(crate) fn category_ref(id: &str) -> String {
    if id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        format!("group:{id}")
    } else {
        super::host::stable_ref("category", id)
    }
}

pub(crate) fn validate_projection(projection: &Projection, provider: &str) -> anyhow::Result<()> {
    let mut references = HashSet::new();
    if matches!(projection.grouping, Grouping::Runtime | Grouping::Container)
        || projection.parents.len() > 8
        || projection.parents.iter().any(|parent| {
            parent.collision_reference.as_ref().is_some_and(|r| {
                !r.starts_with(&format!("group:extensions.{provider}.")) || !clean(r, 256)
            }) || !parent
                .reference
                .starts_with(&format!("group:extensions.{provider}."))
                || !clean(&parent.reference, 256)
                || !references.insert(&parent.reference)
                || !clean(&parent.name, 128)
                || matches!(parent.grouping, Grouping::Runtime | Grouping::Container)
                || parent.evidence.len() > 16
                || parent.evidence.iter().any(|e| !clean(e, 4096))
                || parent.child_name.as_ref().is_some_and(|n| !clean(n, 128))
                || !parent.diagnostics.is_empty()
                || parent.attribution.as_ref().is_some_and(|a| {
                    !matches!(a.source, crate::model::AttributionSource::Extension)
                        || [&a.manifest, &a.bundle_id]
                            .iter()
                            .any(|value| value.as_ref().is_some_and(|v| !clean(v, 128)))
                        || a.inferred
                        || a.via_pid.is_some()
                        || a.extension.as_ref().is_some_and(|e| e.provider != provider)
                        || [&a.path, &a.checkout_path].iter().any(|p| {
                            p.as_ref().is_some_and(|p| {
                                !p.is_absolute()
                                    || p.as_os_str().len() > 4096
                                    || p.to_string_lossy().chars().any(char::is_control)
                            })
                        })
                })
        })
    {
        anyhow::bail!("invalid_placement_projection");
    }
    Ok(())
}

pub(crate) fn normalize_projection(projection: &mut Projection, category: &str) {
    for parent in &mut projection.parents {
        parent.reference =
            super::host::stable_ref(&format!("projection:{category}"), &parent.reference);
        if let Some(reference) = &mut parent.collision_reference {
            *reference = super::host::stable_ref(&format!("projection:{category}"), reference);
        }
    }
}

pub(crate) fn validate_batch(
    proposals: &[super::ClassificationProposal],
    registration: &Registration,
) -> anyhow::Result<()> {
    let mut parents = BTreeMap::new();
    let mut alternate = BTreeMap::new();
    for proposal in proposals {
        if let super::ClassificationEvidence::Tags { proposal } = &proposal.evidence
            && let super::tags::Operation::Add { tag } = &proposal.operation
            && let Some(projection) = &tag.projection
        {
            let ownership = registration
                .ownership
                .iter()
                .find(|o| o.dimension == tag.dimension)
                .ok_or_else(|| anyhow::anyhow!("undeclared_placement_projection"))?;
            for (index, parent) in projection.parents.iter().enumerate() {
                let mut canonical = parent.clone();
                canonical.merge = false;
                canonical.child_name = None;
                let signature = (
                    serde_json::to_string(&canonical)?,
                    projection.parents[index + 1..]
                        .iter()
                        .map(|p| p.reference.clone())
                        .collect::<Vec<_>>(),
                );
                let key = (ownership.category.clone(), parent.reference.clone());
                if parents.get(&key).is_some_and(|old| old != &signature) {
                    anyhow::bail!("colliding_placement_projection");
                }
                parents.insert(key, signature);
                if let Some(reference) = &parent.collision_reference {
                    let key = (ownership.category.clone(), reference.clone());
                    if alternate
                        .get(&key)
                        .is_some_and(|old| old != &parent.reference)
                    {
                        anyhow::bail!("colliding_placement_projection");
                    }
                    alternate.insert(key, parent.reference.clone());
                }
            }
        }
    }
    if alternate.keys().any(|key| parents.contains_key(key)) {
        anyhow::bail!("colliding_placement_projection");
    }
    Ok(())
}
