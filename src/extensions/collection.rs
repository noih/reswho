use super::Contribution;
use crate::{
    containers::{Container, Runtime},
    model::{Diagnostic, ResourceConfiguration},
};
use anyhow::{Result, ensure};

fn text(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}

fn diagnostics(values: &[Diagnostic]) -> bool {
    values.len() <= 128
        && values.iter().all(|d| {
            matches!(
                d.code.as_str(),
                "unavailable"
                    | "permission_denied"
                    | "not_found"
                    | "io_error"
                    | "identity_changed"
                    | "identity_unverified"
                    | "sample_unavailable"
                    | "collection_note"
                    | "collection_failed"
                    | "command_failed"
                    | "invalid_data"
                    | "limit_exceeded"
                    | "unsupported"
                    | "parent_chain_incomplete"
            ) && d.operation.len() <= 128
                && d.operation.starts_with(|c: char| c.is_ascii_lowercase())
                && d.operation
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.".contains(&b))
                && d.message.len() <= 4096
                && !d.message.chars().any(char::is_control)
        })
}
fn configuration(value: Option<&ResourceConfiguration>) -> bool {
    value.is_none_or(|c| text(&c.scope, 64))
}
fn scope(value: Option<&str>) -> bool {
    value.is_none_or(|s| text(s, 64))
}
fn containers(provider: &str, runtime: &Runtime, containers: &[Container]) -> Result<()> {
    ensure!(
        runtime.name == provider
            && matches!(runtime.status.as_str(), "available" | "unavailable")
            && runtime.context.as_deref().is_none_or(|s| text(s, 4096))
            && runtime
                .endpoint_ref
                .as_deref()
                .is_none_or(|s| text(s, 1024))
            && runtime.daemon_id.as_deref().is_none_or(|s| text(s, 256))
            && scope(runtime.metrics_scope.as_deref())
            && configuration(runtime.configuration.as_deref())
            && diagnostics(&runtime.unavailable)
            && containers.len() <= 4096,
        "invalid extension runtime"
    );
    crate::containers::validate_containers(containers)?;
    let mut cpu = 0.0;
    let mut memory = 0u64;
    for container in containers {
        ensure!(
            text(&container.id, 256)
                && container.runtime == runtime.name
                && Some(&container.endpoint_ref) == runtime.endpoint_ref.as_ref()
                && container
                    .cpu_percent
                    .is_none_or(|value| value.is_finite() && value >= 0.0)
                && (!container.is_inactive()
                    || (container.cpu_percent.is_none()
                        && container.memory_bytes.is_none()
                        && container.uptime_seconds.is_none()))
                && scope(container.metrics_scope.as_deref())
                && configuration(container.configuration.as_deref())
                && diagnostics(&container.unavailable),
            "invalid extension container"
        );
        cpu += container.cpu_percent.unwrap_or(0.0);
        memory = memory
            .checked_add(container.memory_bytes.unwrap_or(0))
            .ok_or_else(|| anyhow::anyhow!("extension memory aggregate overflow"))?;
    }
    ensure!(cpu.is_finite(), "extension CPU aggregate overflow");
    Ok(())
}

pub(crate) fn validate(provider: &str, contribution: &Contribution) -> Result<()> {
    match contribution {
        Contribution::Collect {
            runtime,
            containers: values,
        } => containers(provider, runtime, values),
        Contribution::Collected {
            runtime: Some(runtime),
            containers: values,
            ..
        } => containers(provider, runtime, values),
        Contribution::Collected {
            runtime: None,
            containers,
            ..
        } => {
            ensure!(
                containers.is_empty(),
                "extension containers require a runtime"
            );
            Ok(())
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
pub(crate) fn assert_adapter(
    provider: &str,
    result: (Runtime, Vec<Container>),
) -> (Runtime, Vec<Container>) {
    validate(
        provider,
        &Contribution::Collect {
            runtime: result.0.clone(),
            containers: result.1.clone(),
        },
    )
    .unwrap();
    result
}
