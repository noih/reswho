use super::{Capability, Descriptor};
use crate::settings::{Definition, Registry, Resolved, Use};
use anyhow::{Result, bail};
use node_semver::{Range, Version};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
    path::{Path, PathBuf},
};

pub const PROTOCOL_VERSION: u32 = 1;
pub const HOST_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub id: String,
    pub version: String,
    pub engines: BTreeMap<String, String>,
    pub protocol_version: u32,
    pub platforms: Vec<String>,
    pub capabilities: Vec<Capability>,
    pub executable: PathBuf,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub definitions: Vec<Definition>,
    #[serde(default)]
    pub uses: Vec<Use>,
}

impl Manifest {
    pub fn read(path: &Path) -> Result<Self> {
        let mut manifest: Self = parse_file(path)?;
        if !manifest.executable.is_absolute() {
            manifest.executable = path
                .parent()
                .unwrap_or(Path::new("."))
                .join(&manifest.executable);
        }
        manifest.executable = fs::canonicalize(&manifest.executable)
            .map_err(|_| anyhow::anyhow!("extension executable not found"))?;
        manifest.validate()?;
        manifest.validate_entrypoint()?;
        Ok(manifest)
    }
    pub fn validate(&mut self) -> Result<()> {
        if self.id.is_empty()
            || self.id.len() > 64
            || self.version.len() > 64
            || !self
                .id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
            || super::BUILTINS.iter().any(|builtin| {
                use super::Extension;
                builtin.descriptor().id == self.id
            })
        {
            bail!("invalid or reserved extension id");
        }
        let version = self
            .version
            .parse::<Version>()
            .map_err(|_| anyhow::anyhow!("invalid extension SemVer"))?;
        if version.to_string() != self.version {
            bail!("extension version must be full canonical SemVer");
        }
        self.engines
            .get("reswho")
            .ok_or_else(|| anyhow::anyhow!("engines.reswho is required"))?
            .parse::<Range>()
            .map_err(|_| anyhow::anyhow!("invalid engines.reswho npm range"))?;
        if self.platforms.is_empty()
            || self.capabilities.is_empty()
            || self.capabilities.len() > 3
            || self.args.len() > 64
            || !self.executable.is_absolute()
            || self.definitions.len() + self.uses.len() > 256
        {
            bail!("invalid extension manifest limits");
        }
        for definition in &mut self.definitions {
            if !definition.provider.is_empty() && definition.provider != self.id {
                bail!("invalid definition owner");
            }
            definition.provider = self.id.clone();
            definition.version = self.version.clone();
        }
        let mut check = super::settings_registry()?;
        check.register_extension(&self.id, self.definitions.clone(), self.uses.clone())?;
        Ok(())
    }
    pub(super) fn validate_entrypoint(&self) -> Result<()> {
        if !self.args.is_empty() {
            bail!("extension args are unsupported; install a native binary entry point");
        }
        super::native::validate(&self.executable)
    }
    pub fn incompatibility(&self) -> Option<String> {
        if let Err(error) = self.validate_entrypoint() {
            return Some(error.to_string());
        }
        if self.protocol_version != PROTOCOL_VERSION {
            return Some("unsupported protocol version".into());
        }
        if !self
            .platforms
            .iter()
            .any(|platform| platform == std::env::consts::OS)
        {
            return Some("unsupported platform".into());
        }
        let compatible = self
            .engines
            .get("reswho")
            .and_then(|range| range.parse::<Range>().ok())
            .zip(HOST_VERSION.parse::<Version>().ok())
            .is_some_and(|(range, version)| version.satisfies(&range));
        (!compatible).then(|| "CLI version outside engines.reswho".into())
    }
}

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Origin {
    #[default]
    Legacy,
    Explicit {
        manifest: PathBuf,
    },
    DropIn {
        manifest: PathBuf,
    },
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Installed {
    pub manifest: Manifest,
    pub enabled: bool,
    pub removed: bool,
    #[serde(default)]
    pub origin: Origin,
    #[serde(skip)]
    pub problem: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub version: u32,
    pub installed: Vec<Installed>,
    pub shared_definitions: Vec<Definition>,
    #[serde(skip)]
    pub report: Vec<super::report::Record>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            version: 1,
            installed: vec![],
            shared_definitions: vec![],
            report: vec![],
        }
    }
}

impl State {
    pub fn path(config: Option<&Path>) -> Result<PathBuf> {
        let default = crate::settings::default_path();
        Ok(config
            .or(default.as_deref())
            .ok_or_else(|| anyhow::anyhow!("config root unavailable"))?
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
            .join("extensions.json"))
    }
    pub fn load(path: &Path) -> Result<Self> {
        let started = std::time::Instant::now();
        if !path.try_exists()? {
            let mut state = Self::default();
            state.report.push(super::report::Record::new(
                "registry",
                None,
                "read",
                "success",
                Some("state_absent"),
                Some(started),
            ));
            return Ok(state);
        }
        let mut state: Self = parse_file(path)?;
        state.report.push(super::report::Record::new(
            "registry",
            None,
            "read",
            "success",
            None,
            Some(started),
        ));
        if state.version != 1 || state.installed.len() > 128 {
            bail!("unsupported or invalid extension state");
        }
        let mut seen = HashSet::new();
        for entry in &mut state.installed {
            let started = std::time::Instant::now();
            entry.manifest.validate()?;
            state.report.push(super::report::Record::new(
                &entry.manifest.id,
                Some(&entry.manifest.version),
                "validate",
                "success",
                None,
                Some(started),
            ));
            if !seen.insert(&entry.manifest.id) {
                bail!("duplicate installed extension id");
            }
        }
        state.registry()?;
        Ok(state)
    }
    pub fn registry(&self) -> Result<Registry> {
        let mut registry = super::settings_registry()?;
        registry.restore_shared(&self.shared_definitions)?;
        for entry in &self.installed {
            if !entry.removed
                && entry.enabled
                && entry.problem.is_none()
                && entry.manifest.incompatibility().is_none()
            {
                registry.register_extension(
                    &entry.manifest.id,
                    entry.manifest.definitions.clone(),
                    entry.manifest.uses.clone(),
                )?;
            }
        }
        Ok(registry)
    }
    pub fn mutate(path: &Path, operation: impl FnOnce(&mut Self) -> Result<()>) -> Result<()> {
        let parent = path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path.with_extension("lock"))?;
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
            bail!("cannot lock extension state");
        }
        let mut state = Self::load(path)?;
        operation(&mut state)?;
        state.shared_definitions = state.registry()?.shared_definitions();
        let bytes = serde_json::to_vec_pretty(&state)?;
        if bytes.len() > 1_048_576 {
            bail!("extension state exceeds limit");
        }
        let temp = path.with_extension(format!("{}.tmp", std::process::id()));
        struct Remove(PathBuf);
        impl Drop for Remove {
            fn drop(&mut self) {
                let _ = fs::remove_file(&self.0);
            }
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)?;
        let cleanup = Remove(temp.clone());
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)?;
        File::open(parent)?.sync_all()?;
        drop(cleanup);
        Ok(())
    }
    pub fn install(&mut self, mut manifest: Manifest) -> Result<()> {
        manifest.validate()?;
        manifest.validate_entrypoint()?;
        let mut check = super::settings_registry()?;
        check.restore_shared(&self.shared_definitions)?;
        for entry in &self.installed {
            if entry.manifest.id != manifest.id
                && !entry.removed
                && entry.enabled
                && entry.manifest.incompatibility().is_none()
            {
                check.register_extension(
                    &entry.manifest.id,
                    entry.manifest.definitions.clone(),
                    entry.manifest.uses.clone(),
                )?;
            }
        }
        check.register_extension(
            &manifest.id,
            manifest.definitions.clone(),
            manifest.uses.clone(),
        )?;
        if let Some(entry) = self
            .installed
            .iter_mut()
            .find(|entry| entry.manifest.id == manifest.id)
        {
            let enabled = entry.removed || entry.enabled;
            let origin = entry.origin.clone();
            *entry = Installed {
                manifest,
                enabled,
                removed: false,
                origin,
                problem: None,
            };
        } else {
            if self.installed.len() >= 128 {
                bail!("extension install limit reached");
            }
            self.installed.push(Installed {
                manifest,
                enabled: true,
                removed: false,
                origin: Origin::Legacy,
                problem: None,
            });
        }
        Ok(())
    }
    pub fn toggle(&mut self, id: &str, enabled: bool, remove: bool) -> Result<()> {
        let entry = self
            .installed
            .iter_mut()
            .find(|entry| entry.manifest.id == id && !entry.removed)
            .ok_or_else(|| anyhow::anyhow!("extension not installed"))?;
        entry.enabled = enabled;
        entry.removed = remove;
        Ok(())
    }
    pub fn list(&self, settings: &Resolved, containers: bool) -> Vec<Listing> {
        let mut list: Vec<_> = super::list(settings, containers)
            .into_iter()
            .map(|descriptor| Listing {
                explicit_enabled: true,
                compatible: true,
                requires: format!("={HOST_VERSION}"),
                reason: None,
                descriptor,
                report: vec![],
            })
            .collect();
        for entry in self.installed.iter().filter(|entry| !entry.removed) {
            let manifest = &entry.manifest;
            let mut reason = entry
                .problem
                .clone()
                .or_else(|| manifest.incompatibility())
                .or_else(|| {
                    settings
                        .report()
                        .disabled_extensions
                        .get(&manifest.id)
                        .map(|reasons| reasons.join("; "))
                });
            if reason.is_none()
                && !containers
                && manifest
                    .capabilities
                    .iter()
                    .all(|capability| *capability == Capability::Collect)
            {
                reason = Some("container collection disabled by --no-containers".into());
            }
            let enabled = entry.enabled && reason.is_none();
            list.push(Listing {
                descriptor: Descriptor {
                    id: manifest.id.clone(),
                    version: manifest.version.clone(),
                    protocol_version: manifest.protocol_version,
                    source: "external",
                    platforms: manifest.platforms.clone(),
                    capabilities: manifest.capabilities.clone(),
                    enabled,
                    availability: "unknown",
                },
                explicit_enabled: entry.enabled,
                compatible: manifest.incompatibility().is_none(),
                requires: manifest.engines["reswho"].clone(),
                reason,
                report: vec![],
            });
        }
        for record in &self.report {
            if record.issue()
                && !list
                    .iter()
                    .any(|entry| entry.descriptor.id == record.provider)
            {
                list.push(Listing {
                    descriptor: Descriptor {
                        id: record.provider.clone(),
                        version: record.version.clone().unwrap_or("unknown".into()),
                        protocol_version: PROTOCOL_VERSION,
                        source: "drop_in",
                        platforms: vec![],
                        capabilities: vec![],
                        enabled: false,
                        availability: "unknown",
                    },
                    explicit_enabled: false,
                    compatible: false,
                    requires: "unknown".into(),
                    reason: record.reason.clone(),
                    report: vec![],
                });
            }
        }
        for entry in &mut list {
            entry.report = self
                .report
                .iter()
                .filter(|record| record.provider == entry.descriptor.id)
                .cloned()
                .collect();
            for capability in &entry.descriptor.capabilities {
                entry.report.push(super::report::Record::new(
                    &entry.descriptor.id,
                    Some(&entry.descriptor.version),
                    &format!("{capability:?}").to_lowercase(),
                    if entry.descriptor.enabled {
                        "never_invoked"
                    } else {
                        "skipped"
                    },
                    Some(if entry.descriptor.enabled {
                        "not_requested"
                    } else {
                        "not_enabled"
                    }),
                    None,
                ));
            }
        }
        list
    }
}

#[derive(Serialize)]
pub struct Listing {
    #[serde(flatten)]
    pub descriptor: Descriptor,
    pub explicit_enabled: bool,
    pub compatible: bool,
    pub requires: String,
    pub reason: Option<String>,
    pub report: Vec<super::report::Record>,
}

pub(super) fn parse_file<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let mut bytes = Vec::new();
    super::native::open_regular(path)?
        .take(1_048_577)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 1_048_576 {
        bail!("extension metadata exceeds limit");
    }
    serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("invalid extension metadata JSON"))
}

#[cfg(test)]
mod tests;
