use crate::model::Diagnostic;
use serde::Serialize;
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize)]
pub struct Runtime {
    pub name: String,
    pub context: Option<String>,
    pub endpoint_ref: Option<String>,
    pub daemon_id: Option<String>,
    pub status: String,
    pub inventory_complete: bool,
    pub collection_ms: u64,
    pub unavailable: Vec<Diagnostic>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Container {
    pub runtime: String,
    pub endpoint_ref: String,
    pub id: String,
    pub name: String,
    pub state: Option<String>,
    pub started_at: Option<String>,
    pub uptime_seconds: Option<u64>,
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

#[derive(Clone, Debug, Serialize)]
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

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectSource {
    Compose,
    BindWorkdir,
    BindProject,
}

#[derive(Clone, Debug, Serialize)]
pub struct Project {
    pub name: String,
    pub path: Option<PathBuf>,
    pub source: ProjectSource,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ContainerTotals {
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
