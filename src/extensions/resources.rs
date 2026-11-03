use crate::model::{Grouping, Node, Resource, ResourceMetric};
use anyhow::{Result, bail};
use std::collections::HashSet;

fn identifier(value: &str, limit: usize) -> bool {
    !value.is_empty()
        && value.len() <= limit
        && value
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || b"_-./%".contains(&ch))
}

pub(super) fn convert(
    provider: &str,
    version: &str,
    resources: Vec<super::protocol::Resource>,
) -> Result<Vec<Resource>> {
    if resources.len() > 4096 {
        bail!("resource_limit");
    }
    let mut identities = HashSet::new();
    resources
        .into_iter()
        .map(|resource| {
            if !identifier(&resource.id, 256)
                || !identifier(&resource.kind, 64)
                || resource.label.is_empty()
                || resource.label.len() > 128
                || resource
                    .path
                    .as_ref()
                    .is_some_and(|path| !path.is_absolute() || path.as_os_str().len() > 16384)
                || resource.metrics.len() > 64
                || !identities.insert(resource.id.clone())
            {
                bail!("invalid_resource");
            }
            let mut keys = HashSet::new();
            let metrics = resource
                .metrics
                .into_iter()
                .map(|metric| {
                    if !identifier(&metric.key, 128)
                        || !identifier(&metric.unit, 64)
                        || metric.label.is_empty()
                        || metric.label.len() > 128
                        || !keys.insert(metric.key.clone())
                        || metric.value.as_ref().is_some_and(|number| {
                            number.as_f64().is_none_or(|value| !value.is_finite())
                        })
                    {
                        bail!("invalid_resource_metric");
                    }
                    Ok(ResourceMetric {
                        key: metric.key,
                        label: metric.label,
                        unit: metric.unit,
                        value: metric.value,
                    })
                })
                .collect::<Result<_>>()?;
            Ok(Resource {
                provider: provider.into(),
                version: version.into(),
                id: resource.id,
                kind: resource.kind,
                label: resource.label,
                path: resource.path,
                metrics,
            })
        })
        .collect()
}

pub(super) fn build(resources: Vec<Resource>) -> Node {
    let mut root = Node::group(
        "group:resources".into(),
        "Resources".into(),
        vec!["Independent metrics; no cross-resource totals".into()],
        Grouping::Resource,
    );
    for resource in resources {
        let provider_ref = crate::tree::stable_ref("resource-provider", &resource.provider);
        let index = root
            .children
            .iter()
            .position(|node| node.reference == provider_ref)
            .unwrap_or_else(|| {
                root.children.push(Node::group(
                    provider_ref,
                    resource.provider.clone(),
                    vec![],
                    Grouping::Resource,
                ));
                root.children.len() - 1
            });
        let mut node = Node::group(
            crate::tree::stable_ref(
                "resource",
                &format!("{}\0{}", resource.provider, resource.id),
            ),
            resource.label.clone(),
            vec![],
            Grouping::Resource,
        );
        node.kind = "resource";
        node.grouping = None;
        node.resource = Some(resource);
        root.children[index].children.push(node);
    }
    root
}
