use super::*;
use crate::model::{DetailField, ExtensionAttribution, ExtensionDetails};
use serde_json::json;

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
                fields,
            })
        });
        match context.finish_phase(
            id.as_deref(),
            "details",
            validation_started.elapsed(),
            result,
        ) {
            Ok(details) if !details.fields.is_empty() => process.extension_details.push(details),
            Ok(_) => {}
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
    key: String,
    label: String,
    basis: String,
) -> ExtensionAttribution {
    ExtensionAttribution {
        provider,
        key,
        label,
        basis,
    }
}
