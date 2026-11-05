use crate::model::Process;
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

pub(crate) use crate::collect::ArgvPrefix;

pub(crate) const PREFIX_ARGS: usize = 64;

pub(crate) fn entry_setting(provider: &str) -> String {
    format!("extensions.{provider}.entry_argv")
}

pub(crate) fn entry_definition(provider: &str) -> anyhow::Result<crate::settings::Definition> {
    let mut definition: crate::settings::Definition = serde_json::from_value(serde_json::json!({
        "key": entry_setting(provider),
        "type": "boolean",
        "default": true,
        "description": format!("Read the launch arguments of your own {provider} processes once per round to name their entry work"),
        "sensitive": false
    }))?;
    definition.provider = provider.into();
    Ok(definition)
}

pub(crate) fn entries_enabled(settings: &crate::settings::Resolved, provider: &str) -> bool {
    settings
        .get(&entry_setting(provider))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

pub(crate) trait Argv {
    fn read(&mut self, process: &Process) -> Option<Vec<String>>;
}

pub(crate) struct Shared<'a> {
    pub(crate) provider: &'static str,
    pub(crate) reader: &'a Mutex<ArgvPrefix>,
}

impl Argv for Shared<'_> {
    fn read(&mut self, process: &Process) -> Option<Vec<String>> {
        self.reader
            .lock()
            .expect("argv lock")
            .read(self.provider, process, PREFIX_ARGS)
    }
}

pub(crate) fn resolve(value: &str, cwd: Option<&Path>) -> Option<PathBuf> {
    let path = Path::new(value);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd?.join(path)
    };
    path.canonicalize().ok()
}

pub(crate) fn suspicious(stem: &str) -> bool {
    let opaque = crate::collect::sanitize_command(vec!["node".into(), stem.into()], None)
        .redacted_indices
        .contains(&1);
    opaque
        || crate::redaction::sensitive_option(stem)
        || stem
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '+' && c != '/' && c != '=')
            .any(|part| {
                let hex = part.len() >= 16 && part.chars().all(|c| c.is_ascii_hexdigit());
                let base64 = part.len() >= 20
                    && part.chars().any(|c| c.is_ascii_digit())
                    && part.chars().any(|c| c.is_ascii_alphabetic());
                hex || base64
            })
}
