use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};
use std::cmp::Ordering;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueType {
    Integer,
    Number,
    Boolean,
    String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Limits {
    pub min: Option<Number>,
    pub max: Option<Number>,
    pub allowed: Option<Vec<Value>>,
}

pub(crate) fn decimal(number: &Number) -> (bool, String, i64) {
    let text = number.to_string();
    let negative = text.starts_with('-');
    let text = text.trim_start_matches('-');
    let (mantissa, exponent) = text.split_once(['e', 'E']).unwrap_or((text, "0"));
    let exponent = exponent
        .parse::<i64>()
        .unwrap_or(if exponent.starts_with('-') {
            i64::MIN / 2
        } else {
            i64::MAX / 2
        });
    let whole = mantissa.split('.').next().unwrap_or("").len() as i64;
    let digits: String = mantissa.chars().filter(|ch| *ch != '.').collect();
    let leading = digits.bytes().take_while(|byte| *byte == b'0').count();
    let digits = digits[leading..].trim_end_matches('0').to_owned();
    (
        negative && !digits.is_empty(),
        digits,
        exponent
            .saturating_add(whole)
            .saturating_sub(leading as i64),
    )
}

fn numeric_cmp(left: &Number, right: &Number) -> Ordering {
    let (left_negative, left_digits, left_position) = decimal(left);
    let (right_negative, right_digits, right_position) = decimal(right);
    if left_negative != right_negative {
        return right_negative.cmp(&left_negative);
    }
    let ordering = match (left_digits.is_empty(), right_digits.is_empty()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (false, false) => left_position.cmp(&right_position).then_with(|| {
            let length = left_digits.len().max(right_digits.len());
            left_digits
                .bytes()
                .chain(std::iter::repeat(b'0'))
                .take(length)
                .cmp(
                    right_digits
                        .bytes()
                        .chain(std::iter::repeat(b'0'))
                        .take(length),
                )
        }),
    };
    if left_negative {
        ordering.reverse()
    } else {
        ordering
    }
}

fn optional_number_equal(left: &Option<Number>, right: &Option<Number>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => numeric_cmp(left, right) == Ordering::Equal,
        _ => false,
    }
}

fn scalar_equal(left: &Value, right: &Value) -> bool {
    match (left.as_number(), right.as_number()) {
        (Some(left), Some(right)) => numeric_cmp(left, right) == Ordering::Equal,
        _ => left == right,
    }
}

impl PartialEq for Limits {
    fn eq(&self, other: &Self) -> bool {
        optional_number_equal(&self.min, &other.min)
            && optional_number_equal(&self.max, &other.max)
            && match (&self.allowed, &other.allowed) {
                (Some(left), Some(right)) => {
                    left.iter()
                        .all(|value| right.iter().any(|other| scalar_equal(value, other)))
                        && right
                            .iter()
                            .all(|value| left.iter().any(|other| scalar_equal(value, other)))
                }
                (None, None) => true,
                _ => false,
            }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "PublicContract", into = "PublicContract")]
pub struct Contract {
    pub value_type: ValueType,
    pub unit: Option<String>,
    pub limits: Limits,
}

impl Contract {
    pub(crate) fn validate(&self, value: &Value) -> Result<()> {
        let valid_type = match self.value_type {
            ValueType::Integer => {
                value.as_i64().is_some()
                    || value.as_u64().is_some()
                    || value.as_f64().is_some_and(|number| {
                        number.is_finite()
                            && number.fract() == 0.0
                            && number.abs() <= 9_007_199_254_740_992.0
                            && numeric_cmp(
                                value.as_number().expect("numeric value"),
                                &Number::from(number as i64),
                            ) == Ordering::Equal
                    })
            }
            ValueType::Number => value.as_f64().is_some_and(f64::is_finite),
            ValueType::Boolean => value.is_boolean(),
            ValueType::String => value.is_string(),
        };
        if !valid_type {
            bail!("wrong value type");
        }
        if let Some(number) = value.as_number()
            && (self
                .limits
                .min
                .as_ref()
                .is_some_and(|min| numeric_cmp(number, min) == Ordering::Less)
                || self
                    .limits
                    .max
                    .as_ref()
                    .is_some_and(|max| numeric_cmp(number, max) == Ordering::Greater))
        {
            bail!("value outside limits");
        }
        if self
            .limits
            .allowed
            .as_ref()
            .is_some_and(|allowed| !allowed.iter().any(|option| scalar_equal(option, value)))
        {
            bail!("value outside allowed values");
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicContract {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    unit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    options: Option<Vec<Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    min: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    integer: Option<bool>,
}

impl From<Contract> for PublicContract {
    fn from(contract: Contract) -> Self {
        Self {
            kind: match contract.value_type {
                ValueType::Integer | ValueType::Number => "number",
                ValueType::String => "string",
                ValueType::Boolean => "boolean",
            }
            .into(),
            integer: (contract.value_type == ValueType::Integer).then_some(true),
            unit: contract.unit,
            options: contract.limits.allowed,
            min: contract.limits.min,
            max: contract.limits.max,
        }
    }
}

impl TryFrom<PublicContract> for Contract {
    type Error = String;
    fn try_from(public: PublicContract) -> std::result::Result<Self, Self::Error> {
        let value_type = match public.kind.as_str() {
            "number" if public.integer == Some(true) => ValueType::Integer,
            "number" => ValueType::Number,
            "string" => ValueType::String,
            "boolean" => ValueType::Boolean,
            _ => return Err("unsupported setting type".into()),
        };
        if public.integer.is_some_and(|value| !value)
            || (public.integer == Some(true) && value_type != ValueType::Integer)
        {
            return Err("integer must be true and applies only to number".into());
        }
        let contract = Self {
            value_type,
            unit: public.unit,
            limits: Limits {
                min: public.min,
                max: public.max,
                allowed: public.options,
            },
        };
        contract
            .validate_contract()
            .map_err(|error| error.to_string())?;
        Ok(contract)
    }
}

impl Contract {
    fn validate_contract(&self) -> Result<()> {
        if self
            .limits
            .min
            .as_ref()
            .zip(self.limits.max.as_ref())
            .is_some_and(|(min, max)| numeric_cmp(min, max) == Ordering::Greater)
            || self
                .limits
                .min
                .iter()
                .chain(self.limits.max.iter())
                .any(|value| !value.as_f64().is_some_and(f64::is_finite))
            || (!matches!(self.value_type, ValueType::Integer | ValueType::Number)
                && (self.limits.min.is_some() || self.limits.max.is_some()))
        {
            bail!("invalid numeric constraints");
        }
        if let Some(options) = &self.limits.allowed
            && (options.is_empty() || options.iter().any(|option| self.validate(option).is_err()))
        {
            bail!("invalid setting options");
        }
        Ok(())
    }
}

fn unsigned_integer(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| {
        value
            .as_f64()
            .filter(|number| {
                number.is_finite()
                    && *number >= 0.0
                    && number.fract() == 0.0
                    && *number <= 9_007_199_254_740_992.0
            })
            .map(|number| number as u64)
    })
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Definition {
    pub key: String,
    #[serde(flatten)]
    pub contract: Contract,
    pub default: Value,
    pub description: String,
    #[serde(default)]
    pub provider: String,
    pub sensitive: bool,
}

impl Definition {
    pub(crate) fn validate_value(&self, value: &Value) -> Result<()> {
        self.contract.validate(value)?;
        if self.key == "refresh.interval_ms"
            && Instant::now()
                .checked_add(Duration::from_millis(
                    unsigned_integer(value)
                        .ok_or_else(|| anyhow::anyhow!("invalid unsigned interval"))?,
                ))
                .is_none()
        {
            bail!("value cannot be safely scheduled");
        }
        Ok(())
    }
}

#[derive(Clone, Serialize)]
pub struct Use {
    pub key: String,
    #[serde(flatten)]
    pub contract: Contract,
}

impl<'de> Deserialize<'de> for Use {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let mut value = Value::deserialize(deserializer)?;
        let object = value
            .as_object_mut()
            .ok_or_else(|| serde::de::Error::custom("setting use must be an object"))?;
        if object.contains_key("default") {
            return Err(serde::de::Error::custom(
                "setting use cannot declare default",
            ));
        }
        let key = object
            .remove("key")
            .and_then(|key| key.as_str().map(str::to_owned))
            .ok_or_else(|| serde::de::Error::custom("setting use requires key"))?;
        let contract = serde_json::from_value(value).map_err(serde::de::Error::custom)?;
        Ok(Self { key, contract })
    }
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
    pub fn parse_overrides(&self, arguments: &[String]) -> Result<BTreeMap<String, Value>> {
        let mut effective = BTreeMap::new();
        for argument in arguments {
            let (key, value) = argument
                .split_once('=')
                .ok_or_else(|| anyhow::anyhow!("invalid --set: expected KEY=VALUE"))?;
            effective.insert(key, value);
        }
        effective
            .into_iter()
            .map(|(key, raw)| {
                let definition = self
                    .definition(key)
                    .ok_or_else(|| anyhow::anyhow!("{key}: unknown setting"))?;
                let value = if definition.contract.value_type == ValueType::String {
                    Value::String(raw.to_owned())
                } else {
                    serde_json::from_str(raw)
                        .map_err(|_| anyhow::anyhow!("{key}: invalid JSON scalar"))?
                };
                definition
                    .contract
                    .validate(&value)
                    .map_err(|error| anyhow::anyhow!("{key}: {error}"))?;
                Ok((key.to_owned(), value))
            })
            .collect()
    }

    pub fn definitions(&self) -> impl Iterator<Item = &Definition> {
        self.definitions.values()
    }

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
                            min: Some(Number::from(min)),
                            max: Some(Number::from(max)),
                            allowed: None,
                        },
                    },
                    default: Value::from(default),
                    description: description.into(),
                    provider: "core".into(),
                    sensitive: false,
                },
            );
        }
        registry.definitions.insert(
            "core.bundle_version".into(),
            Definition {
                key: "core.bundle_version".into(),
                contract: Contract {
                    value_type: ValueType::Boolean,
                    unit: None,
                    limits: Limits::default(),
                },
                default: Value::Bool(true),
                description: "Read app bundle versions; keep core ownership information".into(),
                provider: "core".into(),
                sensitive: false,
            },
        );
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
            if definition.provider != id || !valid_namespace(key, id) {
                bail!("extension {id}: invalid definition namespace or owner for {key}");
            }
            if definition.contract.validate_contract().is_err() {
                bail!("extension {id}: invalid constraints for {key}");
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

    pub(crate) fn repair_config(&self, config: &mut Config) -> Vec<String> {
        let mut repaired = Vec::new();
        for (key, value) in &mut config.settings {
            if let Some(definition) = self.definition(key)
                && definition.validate_value(value).is_err()
            {
                *value = definition.default.clone();
                repaired.push(key.clone());
            }
        }
        for (key, ids, valid) in [
            (
                "source_priority",
                &mut config.source_priority,
                validate_priority as fn(&[String]) -> Result<()>,
            ),
            (
                "ownership_priority",
                &mut config.ownership_priority,
                validate_ownership_priority,
            ),
            (
                "disabled_builtins",
                &mut config.disabled_builtins,
                validate_priority,
            ),
        ] {
            if valid(ids).is_err() {
                ids.clear();
                repaired.push(key.into());
            }
        }
        repaired
    }

    pub fn resolve(&self, user: &Config, cli: &BTreeMap<String, Value>) -> Result<Resolved> {
        let mut sanitized = user.clone();
        let repaired = self.repair_config(&mut sanitized);
        for key in &repaired {
            eprintln!("reswho: invalid stored setting {key}; using registered default");
        }
        let user = &sanitized;
        validate_priority(&user.source_priority)?;
        validate_ownership_priority(&user.ownership_priority)?;
        validate_priority(&user.disabled_builtins)?;
        use crate::extensions::Extension;
        let mut resolved = Resolved {
            source_priority: user.source_priority.clone(),
            ownership_priority: user.ownership_priority.clone(),
            disabled_builtins: user
                .disabled_builtins
                .iter()
                .filter(|id| {
                    crate::extensions::BUILTINS
                        .iter()
                        .any(|builtin| builtin.descriptor().id == **id)
                })
                .cloned()
                .collect(),
            values: BTreeMap::new(),
            report: Report {
                source_priority: user.source_priority.clone(),
                ownership_priority: user.ownership_priority.clone(),
                disabled_builtins: user.disabled_builtins.clone(),
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
            let validation = definition.validate_value(value);
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
                    source: if repaired.iter().any(|repaired| repaired == key) && source == "user" {
                        "default".into()
                    } else {
                        source.into()
                    },
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

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(skip)]
    pub recovery: Option<Vec<u8>>,
    #[serde(default)]
    pub disabled_builtins: Vec<String>,
    #[serde(default)]
    pub source_priority: Vec<String>,
    #[serde(default)]
    pub ownership_priority: Vec<String>,
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
        use std::io::Read;
        use std::os::unix::fs::OpenOptionsExt;
        let file = match std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)
        {
            Ok(file) => file,
            Err(error) if !explicit && error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => bail!("cannot read config: {error}"),
        };
        if !file.metadata()?.is_file() {
            bail!("config must be a regular file");
        }
        let mut bytes = Vec::new();
        file.take(1_048_577).read_to_end(&mut bytes)?;
        if bytes.len() > 1_048_576 {
            bail!("config exceeds 1 MiB");
        }
        Ok(Self::decode(&bytes))
    }

    pub(crate) fn decode(bytes: &[u8]) -> Self {
        match serde_json::from_slice::<Self>(bytes) {
            Ok(config) if config.version == 1 => config,
            _ => {
                eprintln!(
                    "reswho: malformed or unsupported config; using defaults; Save will preserve the original in a private backup"
                );
                Self {
                    recovery: Some(bytes.to_vec()),
                    ..Self::default()
                }
            }
        }
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
    pub disabled_builtins: Vec<String>,
    pub source_priority: Vec<String>,
    #[serde(default)]
    pub ownership_priority: Vec<String>,
    pub settings: BTreeMap<String, Effective>,
    pub unknown_keys: Vec<String>,
    pub disabled_extensions: BTreeMap<String, Vec<String>>,
}

pub struct Resolved {
    pub source_priority: Vec<String>,
    pub ownership_priority: Vec<String>,
    disabled_builtins: std::collections::HashSet<String>,
    values: BTreeMap<String, Value>,
    report: Report,
}

pub fn validate_priority(ids: &[String]) -> Result<()> {
    let mut seen = std::collections::HashSet::new();
    if ids.len() > 1024
        || ids.iter().any(|id| {
            id.is_empty()
                || id.len() > 64
                || !id
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
                || !seen.insert(id)
        })
    {
        bail!("invalid_source_priority");
    }
    Ok(())
}

pub fn validate_ownership_priority(ids: &[String]) -> Result<()> {
    let mut seen = std::collections::HashSet::new();
    if ids.len() > 1024
        || ids.iter().any(|id| {
            id.is_empty()
                || id.len() > 128
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                || !seen.insert(id)
        })
    {
        bail!("invalid_ownership_priority");
    }
    Ok(())
}

impl Resolved {
    pub(crate) fn contains_sensitive(&self, serialized: &str) -> bool {
        self.report.settings.iter().any(|(key, setting)| {
            (setting.definition.sensitive || crate::redaction::sensitive_option(key))
                && self
                    .get(key)
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|secret| {
                        !secret.is_empty()
                            && serde_json::to_string(secret).is_ok_and(|encoded| {
                                serialized.contains(&encoded[1..encoded.len() - 1])
                            })
                    })
        })
    }
    pub fn set_builtin_enabled(&mut self, id: &str, enabled: bool) -> Result<()> {
        use crate::extensions::Extension;
        if !crate::extensions::BUILTINS
            .iter()
            .any(|builtin| builtin.descriptor().id == id)
        {
            bail!("unknown_builtin");
        }
        if enabled {
            self.disabled_builtins.remove(id);
        } else {
            self.disabled_builtins.insert(id.into());
        }
        self.report
            .disabled_builtins
            .retain(|existing| existing != id);
        if !enabled {
            self.report.disabled_builtins.push(id.into());
        }
        Ok(())
    }

    pub fn is_disabled(&self, id: &str) -> bool {
        self.disabled_builtins.contains(id) || self.report.disabled_extensions.contains_key(id)
    }

    pub fn source_order(&self, ids: impl IntoIterator<Item = String>) -> Vec<String> {
        let mut order = self.source_priority.clone();
        for id in ids {
            if !order.contains(&id) {
                order.push(id);
            }
        }
        order
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.values.get(key)
    }
    pub fn report(&self) -> &Report {
        &self.report
    }
    pub fn interval(&self, key: &str) -> Duration {
        Duration::from_millis(unsigned_integer(&self.values[key]).expect("validated core interval"))
    }
}

#[cfg(test)]
mod tests;

pub mod edit;
