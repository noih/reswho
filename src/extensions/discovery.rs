use super::{
    report::Record,
    state::{Manifest, Origin, State},
};
use anyhow::{Result, bail};
use std::{fs, path::Path, time::Instant};

impl State {
    pub fn startup(path: &Path) -> Result<Self> {
        Self::reconcile_startup(Self::load(path)?, path)
    }

    fn reconcile_startup(mut state: Self, path: &Path) -> Result<Self> {
        let started = Instant::now();
        let before = serde_json::to_vec(&state)?;

        let root = path.parent().unwrap_or(Path::new(".")).join("extensions");
        state.discover(&root);
        state.shared_definitions = state.registry()?.shared_definitions();
        if serde_json::to_vec(&state)? != before {
            let candidate = state.clone();
            if Self::mutate(path, |locked| {
                if serde_json::to_vec(locked)? != before {
                    bail!("registry changed during discovery");
                }
                *locked = candidate;
                Ok(())
            })
            .is_err()
            {
                let mut latest = Self::load(path)?;
                let persisted = latest.installed.clone();
                latest.discover(&root);
                for entry in &mut latest.installed {
                    let unchanged = persisted
                        .iter()
                        .find(|saved| saved.manifest.id == entry.manifest.id)
                        .is_some_and(|saved| {
                            serde_json::to_vec(saved).ok() == serde_json::to_vec(entry).ok()
                        });
                    if !unchanged {
                        entry.problem = Some("persistence_failed".into());
                    }
                }
                state = latest;
                state.report.push(Record::new(
                    "registry",
                    None,
                    "register",
                    "failed",
                    Some("persistence_failed"),
                    Some(started),
                ));
            }
        }
        Ok(state)
    }

    fn discover(&mut self, root: &Path) {
        let started = Instant::now();
        for entry in &mut self.installed {
            if matches!(entry.origin, Origin::DropIn { .. }) {
                entry.problem = Some("package_missing_or_invalid".into());
            }
        }
        let paths = match fs::read_dir(root) {
            Ok(paths) => paths,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(_) => {
                self.report.push(Record::new(
                    "discovery",
                    None,
                    "read",
                    "failed",
                    Some("directory_unreadable"),
                    Some(started),
                ));
                return;
            }
        };
        let mut paths: Vec<_> = paths.take(129).collect();
        if paths.len() > 128 || paths.iter().any(Result::is_err) {
            self.report.push(Record::new(
                "discovery",
                None,
                "read",
                "failed",
                Some("directory_limit_or_read_error"),
                Some(started),
            ));
            return;
        }
        paths.sort_by_key(|entry| entry.as_ref().expect("checked entry").file_name());
        for (index, item) in paths.into_iter().enumerate() {
            let item = item.expect("checked entry");
            let name = item.file_name();
            let name = name.to_str().unwrap_or("");
            let valid_name = !name.is_empty()
                && name.len() <= 64
                && name.chars().all(|c| {
                    c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-')
                });
            let label = if valid_name {
                name.to_string()
            } else {
                format!("invalid-entry-{index}")
            };
            let started = Instant::now();
            let mut phase = "read";
            let mut phase_started = started;
            let mut version = None;
            let result = (|| -> Result<&str> {
                if !valid_name || !item.file_type()?.is_dir() {
                    bail!("invalid_package_directory");
                }
                if self
                    .installed
                    .iter()
                    .any(|entry| entry.manifest.id == name && entry.removed)
                {
                    return Ok("removed");
                }
                let package = fs::canonicalize(item.path())?;
                let manifest_path = package.join("manifest.json");
                if fs::canonicalize(&manifest_path)? != manifest_path {
                    bail!("manifest_path_conflict");
                }
                let mut manifest: Manifest = super::state::parse_file(&manifest_path)
                    .map_err(|_| anyhow::anyhow!("invalid_manifest"))?;
                self.report.push(Record::new(
                    &label,
                    Some(&manifest.version),
                    "read",
                    "success",
                    None,
                    Some(started),
                ));
                phase = "validate";
                let validate_started = Instant::now();
                phase_started = validate_started;
                if manifest.id != name {
                    bail!("folder_id_mismatch");
                }
                if !manifest.executable.is_absolute() {
                    manifest.executable = package.join(&manifest.executable);
                }
                manifest.executable = fs::canonicalize(&manifest.executable)
                    .map_err(|_| anyhow::anyhow!("entrypoint_unavailable"))?;
                if !manifest.executable.starts_with(&package) {
                    bail!("entrypoint_outside_package");
                }
                manifest
                    .validate()
                    .map_err(|_| anyhow::anyhow!("invalid_or_reserved_manifest"))?;
                manifest
                    .validate_entrypoint()
                    .map_err(|_| anyhow::anyhow!("invalid_native_entrypoint"))?;
                version = Some(manifest.version.clone());
                if let Some(entry) = self
                    .installed
                    .iter()
                    .find(|entry| entry.manifest.id == name)
                    && entry.origin
                        != (Origin::DropIn {
                            manifest: manifest_path.clone(),
                        })
                {
                    bail!("ownership_conflict");
                }
                self.report.push(Record::new(
                    &label,
                    version.as_deref(),
                    "validate",
                    "success",
                    None,
                    Some(validate_started),
                ));
                phase = "register";
                let register_started = Instant::now();
                phase_started = register_started;
                self.install(manifest)
                    .map_err(|_| anyhow::anyhow!("registration_conflict_or_limit"))?;
                let entry = self
                    .installed
                    .iter_mut()
                    .find(|entry| entry.manifest.id == name)
                    .expect("installed entry");
                entry.origin = Origin::DropIn {
                    manifest: manifest_path,
                };
                entry.problem = None;
                self.report.push(Record::new(
                    &label,
                    version.as_deref(),
                    "register",
                    "success",
                    None,
                    Some(register_started),
                ));
                Ok("registered")
            })();
            match result {
                Ok("removed") => self.report.push(Record::new(
                    &label,
                    None,
                    "register",
                    "skipped",
                    Some("removed"),
                    None,
                )),
                Ok(_) => {}
                Err(error) => {
                    let text = error.to_string();
                    let reason = match text.as_str() {
                        "invalid_package_directory"
                        | "manifest_path_conflict"
                        | "invalid_manifest"
                        | "folder_id_mismatch"
                        | "entrypoint_unavailable"
                        | "entrypoint_outside_package"
                        | "invalid_or_reserved_manifest"
                        | "invalid_native_entrypoint"
                        | "ownership_conflict"
                        | "registration_conflict_or_limit" => text.as_str(),
                        _ => "package_unavailable",
                    };
                    self.report.push(Record::new(
                        &label,
                        version.as_deref(),
                        phase,
                        "failed",
                        Some(reason),
                        Some(phase_started),
                    ));
                }
            }
        }
    }

    pub fn install_explicit(&mut self, manifest: Manifest, path: &Path) -> Result<()> {
        let path = fs::canonicalize(path)?;
        let origin = self
            .installed
            .iter()
            .find(|entry| entry.manifest.id == manifest.id)
            .map(|entry| entry.origin.clone());
        if let Some(Origin::Explicit { manifest: previous } | Origin::DropIn { manifest: previous }) =
            &origin
            && self
                .installed
                .iter()
                .any(|entry| entry.manifest.id == manifest.id && !entry.removed)
            && previous != &path
        {
            bail!("extension ownership conflict; remove the old registration first");
        }
        let id = manifest.id.clone();
        self.install(manifest)?;
        let entry = self
            .installed
            .iter_mut()
            .find(|entry| entry.manifest.id == id)
            .expect("installed entry");
        entry.origin = if matches!(&origin, Some(Origin::DropIn { manifest }) if manifest == &path)
        {
            origin.expect("drop-in origin")
        } else {
            Origin::Explicit { manifest: path }
        };
        Ok(())
    }

    pub fn registry_reported(&mut self) -> Result<crate::settings::Registry> {
        let mut registry = super::settings_registry()?;
        registry.restore_shared(&self.shared_definitions)?;
        for entry in &mut self.installed {
            let started = Instant::now();
            if entry.removed {
                continue;
            }
            if entry.problem.is_none() {
                entry.problem = entry.manifest.incompatibility();
            }
            self.report.push(Record::new(
                &entry.manifest.id,
                Some(&entry.manifest.version),
                "validate",
                if entry.problem.is_none() {
                    "success"
                } else {
                    "failed"
                },
                entry
                    .problem
                    .as_deref()
                    .map(|_| "incompatible_or_invalid_entrypoint"),
                Some(started),
            ));
            let started = Instant::now();
            if !entry.enabled || entry.problem.is_some() {
                self.report.push(Record::new(
                    &entry.manifest.id,
                    Some(&entry.manifest.version),
                    "register",
                    "skipped",
                    Some("not_enabled"),
                    None,
                ));
                continue;
            }
            let result = registry.register_extension(
                &entry.manifest.id,
                entry.manifest.definitions.clone(),
                entry.manifest.uses.clone(),
            );
            if result.is_err() {
                entry.problem = Some("registration_conflict".into());
            }
            self.report.push(Record::new(
                &entry.manifest.id,
                Some(&entry.manifest.version),
                "register",
                if result.is_ok() { "success" } else { "failed" },
                result.err().as_ref().map(|_| "registration_conflict"),
                Some(started),
            ));
        }
        Ok(registry)
    }

    pub fn initial_report(
        &self,
        settings: &crate::settings::Resolved,
        containers: bool,
    ) -> Vec<Record> {
        let mut report = self.report.clone();
        for id in settings.report().disabled_extensions.keys() {
            report.push(Record::new(
                id,
                self.installed
                    .iter()
                    .find(|entry| entry.manifest.id == *id)
                    .map(|entry| entry.manifest.version.as_str()),
                "register",
                "skipped",
                Some("invalid_settings_or_dependency"),
                None,
            ));
        }
        for entry in self.list(settings, containers) {
            report.extend(entry.report.into_iter().filter(|record| {
                matches!(record.result.as_str(), "never_invoked" | "skipped")
                    && matches!(record.phase.as_str(), "collect" | "classify" | "details")
            }));
        }
        report
    }
}

#[cfg(test)]
mod tests;
