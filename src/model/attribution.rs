use serde::Serialize;
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttributionSource {
    AppBundle,
    BundleId,
    GitRoot,
    GitCommonDir,
    Manifest,
    Executable,
    Compose,
    BindWorkdir,
    BindProject,
}

#[derive(Clone, Debug, Serialize)]
pub struct Attribution {
    pub source: AttributionSource,
    pub path: Option<PathBuf>,
    pub checkout_path: Option<PathBuf>,
    pub manifest: Option<String>,
    pub bundle_id: Option<String>,
    pub inferred: bool,
    pub via_pid: Option<u32>,
}

impl Attribution {
    pub fn new(source: AttributionSource, path: Option<PathBuf>) -> Self {
        Self {
            source,
            path,
            checkout_path: None,
            manifest: None,
            bundle_id: None,
            inferred: false,
            via_pid: None,
        }
    }
}
