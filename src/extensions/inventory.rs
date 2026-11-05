use super::{Capability, Descriptor, report::Record};
use crate::settings::Resolved;
use serde::Serialize;

#[derive(Serialize)]
pub struct Listing {
    #[serde(flatten)]
    pub descriptor: Descriptor,
    #[serde(skip_serializing)]
    pub registration: super::placement::Registration,
    pub explicit_enabled: bool,
    pub reason: Option<String>,
    pub report: Vec<Record>,
}

pub fn list(settings: &Resolved, containers: bool) -> Vec<Listing> {
    super::list(settings, containers)
        .into_iter()
        .map(|descriptor| {
            let reason = settings
                .report()
                .disabled_extensions
                .get(&descriptor.id)
                .map(|reasons| reasons.join("; "))
                .or_else(|| {
                    (!containers && descriptor.capabilities.contains(&Capability::Collect))
                        .then(|| "container collection disabled by --no-containers".into())
                });
            let report = descriptor
                .capabilities
                .iter()
                .map(|capability| {
                    Record::new(
                        &descriptor.id,
                        &format!("{capability:?}").to_lowercase(),
                        if descriptor.enabled {
                            "never_invoked"
                        } else {
                            "skipped"
                        },
                        Some(if descriptor.enabled {
                            "not_requested"
                        } else {
                            "not_enabled"
                        }),
                        None,
                    )
                })
                .collect();
            Listing {
                registration: super::BUILTINS
                    .iter()
                    .find(|p| {
                        use super::Extension;
                        p.descriptor().id == descriptor.id
                    })
                    .map_or_else(Default::default, |p| {
                        use super::Extension;
                        p.registration()
                    }),
                explicit_enabled: !settings.report().disabled_builtins.contains(&descriptor.id),
                descriptor,
                reason,
                report,
            }
        })
        .collect()
}

pub fn initial_report(settings: &Resolved, containers: bool) -> Vec<Record> {
    list(settings, containers)
        .into_iter()
        .flat_map(|entry| entry.report)
        .collect()
}
