use super::{Config, Registry, Resolved};
use anyhow::{Result, bail};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    ffi::CString,
    fs::File,
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Component, Path, PathBuf},
    sync::Arc,
};

pub struct Settings {
    pub registry: Registry,
    pub baseline: Config,
    pub cli: BTreeMap<String, Value>,
    pub path: Option<PathBuf>,
}

impl Settings {
    pub fn new(
        registry: Registry,
        baseline: Config,
        cli: BTreeMap<String, Value>,
        path: Option<PathBuf>,
    ) -> Self {
        Self {
            registry,
            baseline,
            cli,
            path,
        }
    }

    pub fn save(
        &mut self,
        changes: &BTreeMap<String, Option<Value>>,
    ) -> Result<Option<Arc<Resolved>>> {
        self.save_changes(changes, None, None, &BTreeMap::new())
    }

    pub fn save_priority(&mut self, priority: &[String]) -> Result<Option<Arc<Resolved>>> {
        super::validate_priority(priority)?;
        self.save_extensions(Some(priority), None, &BTreeMap::new())
    }

    pub fn save_extensions(
        &mut self,
        source: Option<&[String]>,
        ownership: Option<&[String]>,
        builtins: &BTreeMap<String, bool>,
    ) -> Result<Option<Arc<Resolved>>> {
        let source = source.filter(|ids| *ids != self.baseline.source_priority);
        let ownership = ownership.filter(|ids| *ids != self.baseline.ownership_priority);
        if let Some(ids) = source {
            super::validate_priority(ids)?;
        }
        if let Some(ids) = ownership {
            super::validate_ownership_priority(ids)?;
        }
        let mut check = self.registry.resolve(&self.baseline, &self.cli)?;
        for (id, enabled) in builtins {
            check.set_builtin_enabled(id, *enabled)?;
        }
        self.save_changes(&BTreeMap::new(), source, ownership, builtins)
    }

    fn save_changes(
        &mut self,
        changes: &BTreeMap<String, Option<Value>>,
        priority: Option<&[String]>,
        ownership: Option<&[String]>,
        builtins: &BTreeMap<String, bool>,
    ) -> Result<Option<Arc<Resolved>>> {
        let mut repaired = self.baseline.clone();
        let needs_repair = !self.registry.repair_config(&mut repaired).is_empty()
            || self.baseline.recovery.is_some();
        if !needs_repair
            && changes.is_empty()
            && priority.is_none_or(|ids| ids == self.baseline.source_priority)
            && ownership.is_none_or(|ids| ids == self.baseline.ownership_priority)
            && builtins.is_empty()
        {
            return Ok(None);
        }
        for (key, value) in changes {
            let definition = self
                .registry
                .definition(key)
                .ok_or_else(|| anyhow::anyhow!("Unknown setting; reopen Settings"))?;
            if self.cli.contains_key(key) {
                bail!("CLI override is locked; restart without that override");
            }
            if let Some(value) = value {
                if definition.sensitive && value.as_str() == Some("[redacted]") {
                    bail!("Enter a new sensitive value; redacted placeholders cannot be saved");
                }
                definition.contract.validate(value).map_err(|_| {
                    anyhow::anyhow!("Invalid value; check the selected setting's constraints")
                })?;
            }
        }
        let path = self
            .path
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("No config path; restart with --config PATH"))?;
        let parent = open_parent(path)?;
        let name = CString::new(
            path.file_name()
                .ok_or_else(|| anyhow::anyhow!("Config path must name a file"))?
                .as_bytes(),
        )?;
        let lock_name = CString::new(format!(".{}.settings-lock", name.to_string_lossy()))?;
        let lock = open_at(&parent, &lock_name, libc::O_RDWR | libc::O_CREAT)?;
        safe_file(&lock)?;
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            bail!("Config is being saved elsewhere; retry Save");
        }
        let mut fresh = read_at(&parent, &name)?;
        if fresh.recovery != self.baseline.recovery
            && (fresh.recovery.is_some() || self.baseline.recovery.is_some())
        {
            bail!("Config changed on disk; reopen Settings or Sources before saving");
        }
        if let Some(priority) = priority {
            if fresh.source_priority != self.baseline.source_priority {
                bail!("Priority changed on disk; Esc and reopen Sources before retrying");
            }
            fresh.source_priority = priority.to_vec();
        }

        if let Some(ids) = ownership {
            if fresh.ownership_priority != self.baseline.ownership_priority {
                bail!("Ownership priority changed on disk; Esc and reopen Sources before retrying");
            }
            fresh.ownership_priority = ids.to_vec();
        }
        for (id, enabled) in builtins {
            if fresh.disabled_builtins.contains(id) != self.baseline.disabled_builtins.contains(id)
            {
                bail!("Builtin state changed on disk; Esc and reopen Sources before retrying");
            }
            fresh.disabled_builtins.retain(|existing| existing != id);
            if !enabled {
                fresh.disabled_builtins.push(id.clone());
            }
        }
        for (key, value) in changes {
            if fresh.settings.get(key) != self.baseline.settings.get(key) {
                bail!("Setting changed on disk; Esc and reopen Settings before retrying");
            }
            match value {
                Some(value) => {
                    fresh.settings.insert(key.clone(), value.clone());
                }
                None => {
                    fresh.settings.remove(key);
                }
            }
        }
        self.registry.repair_config(&mut fresh);
        let resolved = Arc::new(self.registry.resolve(&fresh, &self.cli).map_err(|_| {
            anyhow::anyhow!(
                "Config validation failed; repair invalid shared/core settings and retry"
            )
        })?);
        fresh.version = 1;
        let bytes = serde_json::to_vec_pretty(&fresh)?;
        if bytes.len() > 1_048_576 {
            bail!("Config exceeds 1 MiB; reduce it before saving");
        }
        if let Some(original) = &fresh.recovery {
            let backup_name = CString::new(format!(
                ".{}.recovery-{}-{}.bak",
                name.to_string_lossy(),
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_nanos(),
            ))?;
            let mut backup = open_at(
                &parent,
                &backup_name,
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
            )?;
            let backed_up = (|| -> Result<()> {
                backup.write_all(original)?;
                backup.sync_all()?;
                parent.sync_all()?;
                Ok(())
            })();
            if backed_up.is_err() {
                unsafe {
                    libc::unlinkat(parent.as_raw_fd(), backup_name.as_ptr(), 0);
                }
                bail!("Cannot preserve original config backup; Save canceled");
            }
            eprintln!(
                "reswho: original config preserved in {}",
                backup_name.to_string_lossy()
            );
        }
        fresh.recovery = None;
        let temp_name = CString::new(format!(
            ".reswho-settings-{}-{}.tmp",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ))?;
        let mut temp = open_at(
            &parent,
            &temp_name,
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
        )?;
        let result = (|| -> Result<()> {
            temp.write_all(&bytes).map_err(|_| {
                anyhow::anyhow!("Cannot write config; check available space and permissions")
            })?;
            temp.sync_all()
                .map_err(|_| anyhow::anyhow!("Cannot sync config; retry Save"))?;
            if unsafe {
                libc::renameat(
                    parent.as_raw_fd(),
                    temp_name.as_ptr(),
                    parent.as_raw_fd(),
                    name.as_ptr(),
                )
            } != 0
            {
                bail!("Cannot replace config; check directory permissions and retry Save");
            }
            Ok(())
        })();
        unsafe {
            libc::unlinkat(parent.as_raw_fd(), temp_name.as_ptr(), 0);
        }
        result?;
        let _ = parent.sync_all();
        self.baseline = fresh;
        Ok(Some(resolved))
    }

    pub fn reload(&mut self) -> Result<()> {
        let Some(path) = self.path.as_deref() else {
            return Ok(());
        };
        self.baseline = Config::load_path(Some(path), false)?;
        Ok(())
    }
}

fn safe_file(file: &File) -> Result<()> {
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.uid() != unsafe { libc::geteuid() } || metadata.nlink() != 1
    {
        bail!(
            "Config/lock must be a regular file owned by you with one link; repair the path and retry"
        );
    }
    Ok(())
}

fn open_at(parent: &File, name: &CString, flags: i32) -> Result<File> {
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        bail!(
            "Cannot safely open config path; remove symlinks and check permissions before retrying"
        );
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn open_parent(path: &Path) -> Result<File> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut dir = File::open("/")?;
    let parent = absolute
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Config path must name a file"))?;
    for component in parent.components() {
        match component {
            Component::RootDir | Component::CurDir => continue,
            Component::Normal(name) => {
                let name = CString::new(name.as_bytes())?;
                let mut fd = unsafe {
                    libc::openat(
                        dir.as_raw_fd(),
                        name.as_ptr(),
                        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                    )
                };
                if fd < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound
                {
                    if unsafe { libc::mkdirat(dir.as_raw_fd(), name.as_ptr(), 0o700) } != 0
                        && std::io::Error::last_os_error().kind()
                            != std::io::ErrorKind::AlreadyExists
                    {
                        bail!("Cannot create config directory; check permissions");
                    }
                    fd = unsafe {
                        libc::openat(
                            dir.as_raw_fd(),
                            name.as_ptr(),
                            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                        )
                    };
                }
                if fd < 0 {
                    bail!("Config directory is unsafe or unavailable; use a path without symlinks");
                }
                dir = unsafe { File::from_raw_fd(fd) };
            }
            _ => bail!("Config path contains '..'; use an absolute path without parent traversal"),
        }
    }
    let metadata = dir.metadata()?;
    if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o022 != 0 {
        bail!("Config directory must be owned by you and not writable by others");
    }
    Ok(dir)
}

fn read_at(parent: &File, name: &CString) -> Result<Config> {
    let exists = unsafe {
        libc::faccessat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::F_OK,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if exists != 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
        return Ok(Config::default());
    }
    let file = open_at(parent, name, libc::O_RDONLY)?;
    safe_file(&file)?;
    let mut bytes = Vec::new();
    file.take(1_048_577)
        .read_to_end(&mut bytes)
        .map_err(|_| anyhow::anyhow!("Cannot read config; check permissions"))?;
    if bytes.len() > 1_048_576 {
        bail!("Config exceeds 1 MiB; repair it before saving");
    }
    Ok(Config::decode(&bytes))
}
