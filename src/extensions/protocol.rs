use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};
use std::{collections::BTreeMap, path::PathBuf};

pub const FEATURES: &[&str] = &["roles", "resources", "collect_scopes"];

pub fn supported(features: &[String]) -> bool {
    features.len() <= 64
        && features
            .iter()
            .all(|feature| FEATURES.contains(&feature.as_str()))
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub version: String,
    pub engines: BTreeMap<String, String>,
    #[serde(deserialize_with = "integer")]
    pub protocol_version: u32,
    pub platforms: Vec<String>,
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub required_features: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub collect_scopes: Vec<String>,
    pub executable: PathBuf,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default, deserialize_with = "definitions")]
    pub definitions: Vec<crate::settings::Definition>,
    #[serde(default)]
    pub uses: Vec<crate::settings::Use>,
}

#[derive(Serialize, Deserialize)]
pub struct Host {
    pub version: String,
    #[serde(deserialize_with = "integer")]
    pub protocol_version: u32,
    pub features: Vec<String>,
}

#[derive(Serialize, Deserialize)]
pub struct Request {
    #[serde(deserialize_with = "integer")]
    pub protocol_version: u32,
    pub host: Host,
    pub provider: String,
    pub version: String,
    pub capability: super::Capability,
    pub settings: BTreeMap<String, Value>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub collect_scopes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub processes: Option<Vec<ObservedProcess>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process: Option<ObservedProcess>,
}

#[derive(Serialize, Deserialize)]
pub struct ObservedProcess {
    #[serde(deserialize_with = "integer")]
    pub pid: u32,
    #[serde(deserialize_with = "integer")]
    pub started_unix_us: u64,
    #[serde(default, deserialize_with = "optional_integer")]
    pub ppid: Option<u32>,
    #[serde(default, deserialize_with = "optional_integer")]
    pub uid: Option<u32>,
    pub name: String,
    pub executable: Option<PathBuf>,
    pub cwd: Option<PathBuf>,
}

#[derive(Serialize, Deserialize)]
pub struct Response {
    #[serde(deserialize_with = "integer")]
    pub protocol_version: u32,
    pub provider: String,
    pub version: String,
    #[serde(default)]
    pub required_features: Vec<String>,
    pub contribution: Contribution,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "capability", rename_all = "snake_case")]
pub enum Contribution {
    Collect {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        runtime: Option<Runtime>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        containers: Option<Vec<Container>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resources: Option<Vec<Resource>>,
    },
    Classify {
        proposals: Vec<ClassificationProposal>,
    },
    ParsedDetails {
        identity: Identity,
        fields: Vec<DetailField>,
    },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Identity {
    #[serde(deserialize_with = "integer")]
    pub pid: u32,
    #[serde(deserialize_with = "integer")]
    pub started_unix_us: u64,
}

#[derive(Serialize, Deserialize)]
pub struct ClassificationProposal {
    pub identity: Identity,
    pub evidence: ClassificationEvidence,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ClassificationEvidence {
    Custom {
        key: String,
        label: String,
        path: PathBuf,
        basis: String,
    },
    Role {
        key: String,
        label: String,
        path: PathBuf,
        basis: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DetailField {
    pub key: String,
    pub label: String,
    pub value: Value,
    pub sensitive: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Resource {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub path: Option<PathBuf>,
    pub metrics: Vec<Metric>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Metric {
    pub key: String,
    pub label: String,
    pub unit: String,
    #[serde(deserialize_with = "metric_value")]
    pub value: Option<Number>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub operation: String,
    pub message: String,
    #[serde(default, deserialize_with = "optional_integer")]
    pub os_error: Option<i32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Runtime {
    pub name: String,
    pub context: Option<String>,
    pub endpoint_ref: Option<String>,
    pub daemon_id: Option<String>,
    pub status: String,
    pub inventory_complete: bool,
    #[serde(deserialize_with = "integer")]
    pub collection_ms: u64,
    pub unavailable: Vec<Diagnostic>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Container {
    pub runtime: String,
    pub endpoint_ref: String,
    pub id: String,
    pub name: String,
    pub state: Option<String>,
    pub started_at: Option<String>,
    #[serde(default, deserialize_with = "optional_integer")]
    pub uptime_seconds: Option<u64>,
    #[serde(default, deserialize_with = "optional_json_float")]
    pub cpu_percent: Option<f64>,
    #[serde(default, deserialize_with = "optional_integer")]
    pub memory_bytes: Option<u64>,
    pub memory_basis: String,
    pub ports: Option<Vec<Port>>,
    pub mounts: Vec<Mount>,
    pub project: Option<Project>,
    pub candidates: Vec<PathBuf>,
    pub service: Option<String>,
    pub unavailable: Vec<Diagnostic>,
}

fn optional_json_float<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<f64>, D::Error> {
    Option::<serde_json::Number>::deserialize(deserializer)?
        .map(|number| {
            number
                .as_f64()
                .ok_or_else(|| serde::de::Error::custom("expected finite CPU number"))
        })
        .transpose()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Port {
    pub protocol: String,
    #[serde(deserialize_with = "integer")]
    pub container_port: u16,
    pub host_ip: Option<String>,
    #[serde(default, deserialize_with = "optional_integer")]
    pub host_port: Option<u16>,
}

#[derive(Clone, Debug, Default, serde::Deserialize, Serialize)]
pub struct Mount {
    pub kind: String,
    pub source: PathBuf,
    pub destination: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectSource {
    Compose,
    BindWorkdir,
    BindProject,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    pub path: Option<PathBuf>,
    pub source: ProjectSource,
}

impl From<Identity> for super::ProcessIdentity {
    fn from(value: Identity) -> Self {
        Self {
            pid: value.pid,
            started_unix_us: value.started_unix_us,
        }
    }
}

impl From<ClassificationProposal> for super::ClassificationProposal {
    fn from(value: ClassificationProposal) -> Self {
        let evidence = match value.evidence {
            ClassificationEvidence::Custom {
                key,
                label,
                path,
                basis,
            } => super::ClassificationEvidence::Custom {
                key,
                label,
                path,
                basis,
            },
            ClassificationEvidence::Role {
                key,
                label,
                path,
                basis,
            } => super::ClassificationEvidence::Role {
                key,
                label,
                path,
                basis,
            },
        };
        Self {
            identity: value.identity.into(),
            evidence,
        }
    }
}

impl From<DetailField> for crate::model::DetailField {
    fn from(value: DetailField) -> Self {
        Self {
            key: value.key,
            label: value.label,
            value: value.value,
            sensitive: value.sensitive,
        }
    }
}

impl From<Diagnostic> for crate::model::Diagnostic {
    fn from(value: Diagnostic) -> Self {
        Self {
            code: value.code,
            operation: value.operation,
            message: value.message,
            os_error: value.os_error,
        }
    }
}

impl From<Runtime> for crate::containers::Runtime {
    fn from(value: Runtime) -> Self {
        Self {
            name: value.name,
            context: value.context,
            endpoint_ref: value.endpoint_ref,
            daemon_id: value.daemon_id,
            status: value.status,
            inventory_complete: value.inventory_complete,
            collection_ms: value.collection_ms,
            unavailable: value.unavailable.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<Container> for crate::containers::Container {
    fn from(value: Container) -> Self {
        Self {
            runtime: value.runtime,
            endpoint_ref: value.endpoint_ref,
            id: value.id,
            name: value.name,
            state: value.state,
            started_at: value.started_at,
            uptime_seconds: value.uptime_seconds,
            cpu_percent: value.cpu_percent,
            memory_bytes: value.memory_bytes,
            memory_basis: value.memory_basis,
            ports: value
                .ports
                .map(|ports| ports.into_iter().map(Into::into).collect()),
            mounts: value.mounts.into_iter().map(Into::into).collect(),
            project: value.project.map(Into::into),
            candidates: value.candidates,
            service: value.service,
            unavailable: value.unavailable.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<Port> for crate::containers::Port {
    fn from(value: Port) -> Self {
        Self {
            protocol: value.protocol,
            container_port: value.container_port,
            host_ip: value.host_ip,
            host_port: value.host_port,
        }
    }
}

impl From<Mount> for crate::containers::Mount {
    fn from(value: Mount) -> Self {
        Self {
            kind: value.kind,
            source: value.source,
            destination: value.destination,
        }
    }
}

impl From<Project> for crate::containers::Project {
    fn from(value: Project) -> Self {
        Self {
            name: value.name,
            path: value.path,
            source: match value.source {
                ProjectSource::Compose => crate::containers::ProjectSource::Compose,
                ProjectSource::BindWorkdir => crate::containers::ProjectSource::BindWorkdir,
                ProjectSource::BindProject => crate::containers::ProjectSource::BindProject,
            },
        }
    }
}

fn definitions<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<crate::settings::Definition>, D::Error> {
    let values = Vec::<Value>::deserialize(deserializer)?;
    values
        .into_iter()
        .map(|value| {
            let object = value
                .as_object()
                .ok_or_else(|| serde::de::Error::custom("invalid_definition"))?;
            if object.keys().any(|key| {
                !matches!(
                    key.as_str(),
                    "key"
                        | "type"
                        | "unit"
                        | "options"
                        | "min"
                        | "max"
                        | "integer"
                        | "default"
                        | "description"
                        | "provider"
                        | "version"
                        | "sensitive"
                )
            }) {
                return Err(serde::de::Error::custom("unknown_setting_constraint"));
            }
            serde_json::from_value(value)
                .map_err(|_| serde::de::Error::custom("invalid_definition"))
        })
        .collect()
}

fn metric_value<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Number>, D::Error> {
    Option::<Number>::deserialize(deserializer)
}

fn exact_integer<T: TryFrom<i128>>(number: Number) -> Result<T, &'static str> {
    let (negative, digits, position) = crate::settings::decimal(&number);
    if digits.is_empty() {
        return T::try_from(0).map_err(|_| "integer_out_of_range");
    }
    if position < digits.len() as i64 || position > 20 {
        return Err("invalid_integer");
    }
    let mut value = digits.parse::<u64>().map_err(|_| "integer_out_of_range")?;
    for _ in digits.len()..position as usize {
        value = value.checked_mul(10).ok_or("integer_out_of_range")?;
    }
    let value = if negative {
        -i128::from(value)
    } else {
        i128::from(value)
    };
    T::try_from(value).map_err(|_| "integer_out_of_range")
}

fn integer<'de, D: serde::Deserializer<'de>, T: TryFrom<i128>>(
    deserializer: D,
) -> Result<T, D::Error> {
    exact_integer(Number::deserialize(deserializer)?).map_err(serde::de::Error::custom)
}

fn optional_integer<'de, D: serde::Deserializer<'de>, T: TryFrom<i128>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    Option::<Number>::deserialize(deserializer)?
        .map(exact_integer)
        .transpose()
        .map_err(serde::de::Error::custom)
}
