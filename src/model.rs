use serde::Serialize;
use std::path::PathBuf;

#[derive(Clone, Debug, Default, Serialize)]
pub struct Process {
    pub pid: u32,
    pub ppid: Option<u32>,
    pub uid: Option<u32>,
    pub started_unix_us: Option<u64>,
    pub uptime_seconds: Option<u64>,
    pub name: String,
    pub executable: Option<PathBuf>,
    pub cwd: Option<PathBuf>,
    pub cpu_percent: Option<f64>,
    pub resident_bytes: Option<u64>,
    pub network: Option<Network>,
    pub unavailable: Vec<String>,
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

#[derive(Clone, Debug, Default, Serialize)]
pub struct Network {
    pub complete: bool,
    pub sockets: Vec<Socket>,
    pub unavailable: Vec<String>,
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
    pub process: Option<Process>,
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
            process: None,
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

#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    pub schema: &'static str,
    pub captured_unix_ms: u64,
    pub sample_ms: u64,
    pub collection_ms: u64,
    pub system: SystemSummary,
    pub scope: String,
    pub warnings: Vec<String>,
    pub root: Node,
}
