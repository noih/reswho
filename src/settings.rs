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

fn numeric_cmp(left: &Number, right: &Number) -> Ordering {
    fn decimal(number: &Number) -> (bool, String, i64) {
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
    #[serde(default)]
    pub version: String,
    pub sensitive: bool,
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

    pub fn restore_shared(&mut self, definitions: &[Definition]) -> Result<()> {
        let mut pending = self.definitions.clone();
        for definition in definitions {
            if !definition.key.starts_with("shared.") {
                bail!("invalid canonical definition namespace");
            }
            let mut check = Self::core();
            check.register_extension(&definition.provider, vec![definition.clone()], vec![])?;
            if let Some(existing) = pending.get(&definition.key) {
                if existing.contract != definition.contract
                    || existing.sensitive != definition.sensitive
                {
                    bail!("incompatible canonical definition");
                }
            } else {
                pending.insert(definition.key.clone(), definition.clone());
            }
        }
        self.definitions = pending;
        Ok(())
    }

    pub fn shared_definitions(&self) -> Vec<Definition> {
        self.definitions
            .values()
            .filter(|definition| definition.key.starts_with("shared."))
            .cloned()
            .collect()
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
                            unsigned_integer(value)
                                .ok_or_else(|| anyhow::anyhow!("invalid unsigned interval"))?,
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

#[derive(Clone, Default, Serialize, Deserialize)]
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
        Duration::from_millis(unsigned_integer(&self.values[key]).expect("validated core interval"))
    }
}

#[cfg(test)]
mod tests;

pub mod edit;
