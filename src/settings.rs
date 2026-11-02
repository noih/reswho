use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueType {
    Integer,
    Boolean,
    String,
}

#[derive(Clone, Debug, Eq, Default, Serialize)]
pub struct Limits {
    pub min: Option<u64>,
    pub max: Option<u64>,
    pub allowed: Option<Vec<Value>>,
}

impl PartialEq for Limits {
    fn eq(&self, other: &Self) -> bool {
        self.min == other.min
            && self.max == other.max
            && match (&self.allowed, &other.allowed) {
                (Some(left), Some(right)) => {
                    left.iter().all(|value| right.contains(value))
                        && right.iter().all(|value| left.contains(value))
                }
                (None, None) => true,
                _ => false,
            }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Contract {
    pub value_type: ValueType,
    pub unit: Option<String>,
    pub limits: Limits,
}

impl Contract {
    fn validate(&self, value: &Value) -> Result<()> {
        let valid_type = match self.value_type {
            ValueType::Integer => value.as_u64().is_some(),
            ValueType::Boolean => value.is_boolean(),
            ValueType::String => value.is_string(),
        };
        if !valid_type {
            bail!("wrong value type");
        }
        if let Some(number) = value.as_u64()
            && (self.limits.min.is_some_and(|min| number < min)
                || self.limits.max.is_some_and(|max| number > max))
        {
            bail!("value outside limits");
        }
        if self
            .limits
            .allowed
            .as_ref()
            .is_some_and(|allowed| !allowed.contains(value))
        {
            bail!("value outside allowed values");
        }
        Ok(())
    }
}

#[derive(Clone, Serialize)]
pub struct Definition {
    pub key: String,
    #[serde(flatten)]
    pub contract: Contract,
    pub default: Value,
    pub description: String,
    pub provider: String,
    pub version: String,
    pub sensitive: bool,
}

#[derive(Clone)]
pub struct Use {
    pub key: String,
    pub contract: Contract,
}

pub struct Registry {
    definitions: BTreeMap<String, Definition>,
    uses: BTreeMap<String, Vec<Use>>,
}

fn core_key(key: &str) -> bool {
    matches!(key, "sampling.interval_ms" | "refresh.interval_ms")
}

fn valid_namespace(key: &str, provider: &str) -> bool {
    let valid_parts = key.split('.').all(|part| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    });
    valid_parts
        && (core_key(key)
            || key.starts_with("shared.")
            || key.starts_with(&format!("extensions.{provider}.")))
}

impl Registry {
    pub fn definition(&self, key: &str) -> Option<&Definition> {
        self.definitions.get(key)
    }

    pub fn core() -> Self {
        let mut registry = Self {
            definitions: BTreeMap::new(),
            uses: BTreeMap::new(),
        };
        for (key, default, min, max, description) in [
            (
                "sampling.interval_ms",
                250,
                200,
                5000,
                "Minimum paired CPU sampling wait",
            ),
            (
                "refresh.interval_ms",
                2000,
                1,
                u64::MAX,
                "TUI wait after receiving a collection result",
            ),
        ] {
            registry.definitions.insert(
                key.into(),
                Definition {
                    key: key.into(),
                    contract: Contract {
                        value_type: ValueType::Integer,
                        unit: Some("ms".into()),
                        limits: Limits {
                            min: Some(min),
                            max: Some(max),
                            allowed: None,
                        },
                    },
                    default: Value::from(default),
                    description: description.into(),
                    provider: "core".into(),
                    version: "1".into(),
                    sensitive: false,
                },
            );
        }
        registry
    }

    pub fn register_extension(
        &mut self,
        id: &str,
        definitions: Vec<Definition>,
        uses: Vec<Use>,
    ) -> Result<()> {
        if id.is_empty()
            || id == "core"
            || !id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            || self.uses.contains_key(id)
        {
            bail!("invalid or duplicate extension id");
        }
        let mut pending = self.definitions.clone();
        for definition in definitions {
            let key = &definition.key;
            if definition.provider != id
                || definition.version.is_empty()
                || !valid_namespace(key, id)
            {
                bail!("extension {id}: invalid definition namespace or owner for {key}");
            }
            if definition
                .contract
                .limits
                .min
                .zip(definition.contract.limits.max)
                .is_some_and(|(min, max)| min > max)
                || (definition.contract.value_type != ValueType::Integer
                    && (definition.contract.limits.min.is_some()
                        || definition.contract.limits.max.is_some()))
            {
                bail!("extension {id}: invalid limits for {key}");
            }
            if definition.contract.validate(&definition.default).is_err() {
                bail!("extension {id}: invalid default for {key}");
            }
            if let Some(existing) = pending.get(key) {
                if existing.contract != definition.contract
                    || existing.sensitive != definition.sensitive
                {
                    bail!("extension {id}: incompatible definition for {key}");
                }
            } else {
                pending.insert(key.clone(), definition);
            }
        }
        for usage in &uses {
            if !valid_namespace(&usage.key, id) {
                bail!("extension {id}: invalid use namespace for {}", usage.key);
            }
        }
        self.definitions = pending;
        self.uses.insert(id.into(), uses);
        Ok(())
    }

    pub fn resolve_uses(&self) -> BTreeMap<String, Vec<String>> {
        let mut errors = BTreeMap::new();
        for (id, uses) in &self.uses {
            for usage in uses {
                let reason = match self.definitions.get(&usage.key) {
                    None => Some("missing definition"),
                    Some(definition) if definition.contract != usage.contract => {
                        Some("incompatible use")
                    }
                    Some(_) => None,
                };
                if let Some(reason) = reason {
                    errors
                        .entry(id.clone())
                        .or_insert_with(Vec::new)
                        .push(format!("{}: {reason}", usage.key));
                }
            }
        }
        errors
    }

    pub fn resolve(&self, user: &Config, cli: &BTreeMap<String, Value>) -> Result<Resolved> {
        let mut resolved = Resolved {
            values: BTreeMap::new(),
            report: Report {
                settings: BTreeMap::new(),
                unknown_keys: user
                    .settings
                    .keys()
                    .filter(|key| !self.definitions.contains_key(*key))
                    .cloned()
                    .collect(),
                disabled_extensions: self.resolve_uses(),
            },
        };
        for (key, definition) in &self.definitions {
            let (value, source) = if let Some(value) = cli.get(key) {
                (value, "cli")
            } else if let Some(value) = user.settings.get(key) {
                (value, "user")
            } else {
                (&definition.default, "default")
            };
            let validation = definition.contract.validate(value).and_then(|()| {
                if key == "refresh.interval_ms"
                    && Instant::now()
                        .checked_add(Duration::from_millis(
                            value.as_u64().expect("validated integer"),
                        ))
                        .is_none()
                {
                    bail!("value cannot be safely scheduled");
                }
                Ok(())
            });
            if let Err(error) = validation {
                if key.starts_with("extensions.") {
                    let id = key.split('.').nth(1).expect("validated namespace");
                    resolved
                        .report
                        .disabled_extensions
                        .entry(id.into())
                        .or_default()
                        .push(format!("{key}: {error}"));
                    continue;
                }
                bail!("{key}: {error}");
            }
            resolved.values.insert(key.clone(), value.clone());
            resolved.report.settings.insert(
                key.clone(),
                Effective {
                    value: if definition.sensitive {
                        Value::String("[redacted]".into())
                    } else {
                        value.clone()
                    },
                    source: source.into(),
                    definition: {
                        let mut public = definition.clone();
                        if public.sensitive {
                            public.default = Value::String("[redacted]".into());
                            public.contract.limits.allowed = None;
                        }
                        public
                    },
                },
            );
        }
        resolved.values.retain(|key, _| {
            !key.strip_prefix("extensions.")
                .and_then(|key| key.split('.').next())
                .is_some_and(|id| resolved.report.disabled_extensions.contains_key(id))
        });
        resolved
            .report
            .settings
            .retain(|key, _| resolved.values.contains_key(key));
        Ok(resolved)
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::core()
    }
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    pub settings: BTreeMap<String, Value>,
}

impl Config {
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let default_path = default_path();
        Self::load_path(path.or(default_path.as_deref()), path.is_some())
    }

    pub fn load_path(path: Option<&Path>, explicit: bool) -> Result<Self> {
        let Some(path) = path else {
            return Ok(Self::default());
        };
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if !explicit && error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => bail!("cannot read config: {error}"),
        };
        let config: Self = serde_json::from_slice(&bytes).map_err(|error| {
            anyhow::anyhow!(
                "invalid config JSON at line {} column {}",
                error.line(),
                error.column()
            )
        })?;
        if config.version != 1 {
            bail!("unsupported config version (expected 1)");
        }
        Ok(config)
    }
}

pub fn default_path() -> Option<PathBuf> {
    path_from_env(
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("HOME"),
    )
}

fn path_from_env(
    xdg: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> Option<PathBuf> {
    xdg.filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.map(|home| PathBuf::from(home).join(".config")))
        .map(|base| base.join("reswho/config.json"))
}

#[derive(Serialize)]
pub struct Effective {
    pub value: Value,
    pub source: String,
    pub definition: Definition,
}

#[derive(Serialize)]
pub struct Report {
    pub settings: BTreeMap<String, Effective>,
    pub unknown_keys: Vec<String>,
    pub disabled_extensions: BTreeMap<String, Vec<String>>,
}

pub struct Resolved {
    values: BTreeMap<String, Value>,
    report: Report,
}

impl Resolved {
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.values.get(key)
    }
    pub fn report(&self) -> &Report {
        &self.report
    }
    pub fn interval(&self, key: &str) -> Duration {
        Duration::from_millis(self.values[key].as_u64().expect("validated core interval"))
    }
}

#[cfg(test)]
mod tests;
