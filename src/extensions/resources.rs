use crate::model::{Grouping, Node, Resource};

pub(super) fn build(resources: Vec<Resource>) -> Node {
    let mut root = Node::group(
        "group:resources".into(),
        "Resources".into(),
        vec!["Independent metrics; no cross-resource totals".into()],
        Grouping::Resource,
    );
    root.category_order = Some(350);
    for resource in resources {
        let provider_ref =
            crate::extensions::host::stable_ref("resource-provider", &resource.provider);
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
            crate::extensions::host::stable_ref(
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

pub(super) fn validate(provider: &str, resources: &[Resource]) -> anyhow::Result<()> {
    use anyhow::bail;
    use std::collections::HashSet;
    let identifier = |value: &str, limit| {
        !value.is_empty()
            && value.len() <= limit
            && value
                .bytes()
                .all(|ch| ch.is_ascii_alphanumeric() || b"_-./%".contains(&ch))
    };
    if resources.len() > 4096 {
        bail!("resource_limit");
    }
    let mut identities = HashSet::new();
    for resource in resources {
        if resource.provider != provider
            || !identifier(&resource.id, 256)
            || !identifier(&resource.kind, 64)
            || resource.label.is_empty()
            || resource.label.len() > 128
            || resource
                .path
                .as_ref()
                .is_some_and(|path| !path.is_absolute() || path.as_os_str().len() > 16384)
            || resource.metrics.len() > 64
            || !identities.insert(&resource.id)
        {
            bail!("invalid_resource");
        }
        let mut keys = HashSet::new();
        for metric in &resource.metrics {
            if !identifier(&metric.key, 128)
                || !identifier(&metric.unit, 64)
                || metric.label.is_empty()
                || metric.label.len() > 128
                || !keys.insert(&metric.key)
                || metric
                    .value
                    .as_ref()
                    .is_some_and(|number| number.as_f64().is_none_or(|value| !value.is_finite()))
            {
                bail!("invalid_resource_metric");
            }
        }
    }
    Ok(())
}
