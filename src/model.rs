use serde::{Deserialize, Serialize};
use std::path::PathBuf;

mod attribution;
mod diagnostic;
pub use attribution::{Attribution, AttributionSource};
pub use diagnostic::Diagnostic;

pub const JSON_SCHEMA_VERSION: &str = "reswho/12";

#[derive(Clone, Debug, Default, Serialize)]
pub struct Process {
    #[serde(skip)]
    pub roles: Vec<Role>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag_report: Option<crate::extensions::tags::Report>,
    pub pid: u32,
    pub ppid: Option<u32>,
    pub uid: Option<u32>,
    pub started_unix_us: Option<u64>,
    pub uptime_seconds: Option<u64>,
    pub name: String,
    pub executable: Option<PathBuf>,
    pub cwd: Option<PathBuf>,
    pub command: Option<ProcessCommand>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub extension_details: Vec<ExtensionDetails>,
    pub cpu_percent: Option<f64>,
    pub resident_bytes: Option<u64>,
    pub network: Option<Network>,
    pub unavailable: Vec<Diagnostic>,
}

impl Process {
    pub(crate) fn is_unreadable(&self) -> bool {
        self.ppid.is_none()
            && self.uid.is_none()
            && self.started_unix_us.is_none()
            && self.uptime_seconds.is_none()
            && self.executable.is_none()
            && self.cwd.is_none()
            && self.cpu_percent.is_none()
            && self.resident_bytes.is_none()
            && self.network.is_none()
            && (self.name.is_empty() || self.name == format!("process {}", self.pid))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Role {
    pub provider: String,
    pub version: String,
    pub key: String,
    pub label: String,
    pub basis: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Resource {
    pub provider: String,
    pub version: String,
    pub id: String,
    pub kind: String,
    pub label: String,
    pub path: Option<PathBuf>,
    pub metrics: Vec<ResourceMetric>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResourceMetric {
    pub key: String,
    pub label: String,
    pub unit: String,
    pub value: Option<serde_json::Number>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetailField {
    pub key: String,
    pub label: String,
    pub value: serde_json::Value,
    pub sensitive: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExtensionDetails {
    pub provider: String,
    pub version: String,
    pub fields: Vec<DetailField>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExtensionAttribution {
    pub provider: String,
    pub version: String,
    pub key: String,
    pub label: String,
    pub basis: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProcessCommand {
    pub argv: Option<Vec<String>>,
    pub redacted_indices: Vec<usize>,
    pub entrypoint: Option<String>,
    pub unavailable: Vec<Diagnostic>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Network {
    pub complete: bool,
    pub sockets: Vec<Socket>,
    pub unavailable: Vec<Diagnostic>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Tcp,
    Udp,
}

#[derive(Clone, Debug, Serialize)]
pub struct Socket {
    pub fd: i32,
    pub protocol: Protocol,
    pub local: std::net::SocketAddr,
    pub remote: Option<std::net::SocketAddr>,
    pub tcp_state: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Totals {
    pub processes: usize,
    #[serde(rename = "known_cpu_percent")]
    pub cpu_percent: f64,
    #[serde(rename = "known_resident_bytes")]
    pub resident_bytes: u64,
    pub cpu_unknown: usize,
    pub memory_unknown: usize,
    pub oldest_uptime_seconds: Option<u64>,
    pub uptime_unknown: usize,
}

impl Totals {
    pub fn add_process(&mut self, process: &Process) {
        self.processes += 1;
        match process.cpu_percent {
            Some(cpu) => self.cpu_percent += cpu,
            None => self.cpu_unknown += 1,
        }
        match process.resident_bytes {
            Some(memory) => self.resident_bytes += memory,
            None => self.memory_unknown += 1,
        }
        self.oldest_uptime_seconds = self.oldest_uptime_seconds.max(process.uptime_seconds);
        self.uptime_unknown += usize::from(process.uptime_seconds.is_none());
    }

    pub fn add(&mut self, other: &Self) {
        self.processes += other.processes;
        self.cpu_percent += other.cpu_percent;
        self.resident_bytes += other.resident_bytes;
        self.cpu_unknown += other.cpu_unknown;
        self.memory_unknown += other.memory_unknown;
        self.oldest_uptime_seconds = self.oldest_uptime_seconds.max(other.oldest_uptime_seconds);
        self.uptime_unknown += other.uptime_unknown;
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Grouping {
    Category,
    Resource,
    Runtime,
    Container,
    App,
    Repo,
    Checkout,
    Project,
    Bin,
    System,
    Unknown,
}

impl Grouping {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Category => "",
            Self::Resource => "resource",
            Self::Runtime => "runtime",
            Self::Container => "container",
            Self::App => "app",
            Self::Repo => "repo",
            Self::Checkout => "checkout",
            Self::Project => "project",
            Self::Bin => "bin",
            Self::System => "system",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Node {
    pub reference: String,
    pub name: String,
    pub kind: &'static str,
    pub grouping: Option<Grouping>,
    pub evidence: Vec<String>,
    pub attribution: Option<Attribution>,
    pub diagnostics: Vec<Diagnostic>,
    pub process: Option<Process>,
    pub resource: Option<Resource>,
    pub container: Option<crate::containers::Container>,
    pub runtime: Option<crate::containers::Runtime>,
    pub container_totals: Option<crate::containers::ContainerTotals>,
    pub totals: Totals,
    pub children: Vec<Node>,
    pub omitted_children: usize,
}

impl Node {
    pub fn group(
        reference: String,
        name: String,
        evidence: Vec<String>,
        grouping: Grouping,
    ) -> Self {
        Self {
            reference,
            name,
            kind: "group",
            grouping: Some(grouping),
            evidence,
            attribution: None,
            diagnostics: Vec::new(),
            process: None,
            resource: None,
            container: None,
            runtime: None,
            container_totals: None,
            totals: Totals::default(),
            children: Vec::new(),
            omitted_children: 0,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct SystemSummary {
    pub cpu_percent: Option<f64>,
    pub logical_cpus: Option<usize>,
    pub memory_total_bytes: Option<u64>,
    pub memory_used_bytes: Option<u64>,
    pub swap_used_bytes: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub extensions: Vec<crate::extensions::report::Record>,
    pub schema: &'static str,
    pub captured_unix_ms: u64,
    pub sample_ms: u64,
    pub collection_ms: u64,
    pub system: SystemSummary,
    pub scope: String,
    pub warnings: Vec<Diagnostic>,
    pub root: Node,
}

impl Snapshot {
    pub fn project(&mut self, detail_pid: Option<u32>) {
        project_node(&mut self.root, detail_pid);
    }
}

pub fn project_node(node: &mut Node, detail_pid: Option<u32>) {
    if let Some(process) = &mut node.process {
        process.roles.clear();
        if Some(process.pid) != detail_pid {
            process.tag_report = None;
            process.extension_details.clear();
            process.command = None;
            node.evidence.clear();
            node.attribution = None;
        } else if process.tag_report.is_none() {
            process.tag_report = Some(crate::extensions::tags::Report {
                diagnostics: vec![Diagnostic::new(
                    "unavailable",
                    "tags",
                    "Tag evidence was not collected for this process",
                )],
                ..Default::default()
            });
        }
    } else if node.container.is_none()
        && node.container_totals.is_none()
        && node.runtime.is_none()
        && node.resource.is_none()
    {
        let path = node.attribution.as_ref().and_then(|a| a.path.clone());
        let external = node
            .attribution
            .as_ref()
            .is_some_and(|a| matches!(a.source, AttributionSource::Extension));
        node.evidence.retain(|value| {
            [
                "Application bundle: ",
                "Git root from working directory: ",
                "project ",
                "exe ",
                "repo ",
                "bundle ",
                "compose ",
                "Repository metadata unavailable: ",
                "System executable location",
            ]
            .iter()
            .any(|prefix| value.starts_with(prefix))
        });
        if node.evidence.is_empty()
            && let Some(path) = path
        {
            node.evidence.push(format!("project {}", path.display()));
        }
        if external {
            node.attribution = None;
        }
    }
    for child in &mut node.children {
        project_node(child, detail_pid);
    }
}

impl Serialize for Snapshot {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut root = self.root.clone();
        let detail_pid = self
            .scope
            .strip_prefix("pid:")
            .and_then(|pid| pid.parse().ok());
        project_node(&mut root, detail_pid);
        let mut state = serializer.serialize_struct("Snapshot", 9)?;
        state.serialize_field("extensions", &self.extensions)?;
        state.serialize_field("schema", &self.schema)?;
        state.serialize_field("captured_unix_ms", &self.captured_unix_ms)?;
        state.serialize_field("sample_ms", &self.sample_ms)?;
        state.serialize_field("collection_ms", &self.collection_ms)?;
        state.serialize_field("system", &self.system)?;
        state.serialize_field("scope", &self.scope)?;
        state.serialize_field("warnings", &self.warnings)?;
        state.serialize_field("root", &root)?;
        state.end()
    }
}
