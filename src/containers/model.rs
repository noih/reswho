use crate::model::Diagnostic;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Runtime {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metrics_scope: Option<String>,
    #[serde(skip)]
    pub host_scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configuration: Option<Box<crate::model::ResourceConfiguration>>,
    pub name: String,
    pub context: Option<String>,
    pub endpoint_ref: Option<String>,
    pub daemon_id: Option<String>,
    pub status: String,
    pub inventory_complete: bool,
    pub collection_ms: u64,
    pub unavailable: Vec<Diagnostic>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Container {
    #[serde(skip)]
    pub host_scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configuration: Option<Box<crate::model::ResourceConfiguration>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metrics_scope: Option<String>,
    pub runtime: String,
    pub endpoint_ref: String,
    pub id: String,
    pub name: String,
    pub state: Option<String>,
    pub started_at: Option<String>,
    pub uptime_seconds: Option<u64>,
    #[serde(default, deserialize_with = "optional_json_float")]
    pub cpu_percent: Option<f64>,
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

impl Container {
    pub fn is_inactive(&self) -> bool {
        matches!(
            self.state.as_deref(),
            Some("created" | "exited" | "dead" | "stopped")
        )
    }

    pub fn is_active(&self) -> bool {
        matches!(
            self.state.as_deref(),
            Some("running" | "paused" | "restarting")
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Port {
    pub protocol: String,
    pub container_port: u16,
    pub host_ip: Option<String>,
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

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ContainerTotals {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metrics_scope: Option<Box<str>>,
    pub containers: usize,
    pub active_containers: usize,
    pub inactive_containers: usize,
    pub activity_unknown: usize,
    pub inventory_complete: bool,
    pub known_cpu_percent: f64,
    pub known_memory_bytes: u64,
    pub cpu_unknown: usize,
    pub memory_unknown: usize,
    pub oldest_uptime_seconds: Option<u64>,
    pub uptime_unknown: usize,
}

impl ContainerTotals {
    pub fn add(&mut self, container: &Container) {
        if self.containers == 0 {
            self.metrics_scope = container.metrics_scope.as_deref().map(Into::into);
        } else if self.metrics_scope.as_deref() != container.metrics_scope.as_deref() {
            self.metrics_scope = None;
        }
        self.containers += 1;
        if container.is_inactive() {
            self.inactive_containers += 1;
            return;
        }
        if container.is_active() {
            self.active_containers += 1;
        } else {
            self.activity_unknown += 1;
        }
        self.known_cpu_percent += container.cpu_percent.unwrap_or(0.0);
        self.known_memory_bytes = self
            .known_memory_bytes
            .saturating_add(container.memory_bytes.unwrap_or(0));
        self.cpu_unknown += usize::from(container.cpu_percent.is_none());
        self.memory_unknown += usize::from(container.memory_bytes.is_none());
        self.oldest_uptime_seconds = self.oldest_uptime_seconds.max(container.uptime_seconds);
        self.uptime_unknown += usize::from(container.uptime_seconds.is_none());
    }
}
