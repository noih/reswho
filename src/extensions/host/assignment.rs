use crate::model::{Attribution, AttributionSource, Process};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub(crate) struct Assignment {
    pub(crate) group_key: Option<String>,
    pub(crate) category: String,
    pub(crate) category_name: String,
    pub(crate) key: String,
    pub(crate) name: String,
    pub(crate) evidence: String,
    pub(crate) projection: Option<crate::extensions::placement::Projection>,
    pub(crate) attribution: Option<Attribution>,
}

pub(crate) fn system_executable(path: Option<&Path>) -> bool {
    path.is_some_and(|path| {
        ["/System", "/usr/libexec", "/usr/sbin", "/sbin"]
            .iter()
            .any(|root| path.starts_with(root))
    })
}

pub(crate) fn system(evidence: &str) -> Assignment {
    Assignment {
        group_key: None,
        category: "system".into(),
        category_name: "System".into(),
        key: "services".into(),
        name: "System services".into(),
        evidence: evidence.into(),
        projection: None,
        attribution: None,
    }
}

pub(crate) fn unknown(evidence: &str) -> Assignment {
    Assignment {
        group_key: None,
        category: "unclassified".into(),
        category_name: "Unclassified".into(),
        key: "unknown".into(),
        name: "Unknown ownership".into(),
        evidence: evidence.into(),
        projection: None,
        attribution: None,
    }
}

pub(crate) fn direct(process: &Process, bundle: Option<&PathBuf>) -> Assignment {
    if let Some(bundle) = bundle {
        let key = bundle.to_string_lossy().into_owned();
        let filename = bundle.file_name().unwrap_or_default().to_string_lossy();
        let name = filename
            .trim_end_matches(".bundle")
            .trim_end_matches(".app")
            .to_owned();
        return Assignment {
            group_key: None,
            category: "applications".into(),
            category_name: "Applications".into(),
            name,
            evidence: format!("Application bundle: {key}"),
            attribution: Some(Attribution::new(
                AttributionSource::AppBundle,
                Some(bundle.clone()),
            )),
            key,
            projection: None,
        };
    }
    if system_executable(process.executable.as_deref()) {
        return system("System executable location");
    }
    unknown("No application identified from available paths")
}
