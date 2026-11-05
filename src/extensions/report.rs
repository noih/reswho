use serde::Serialize;
use std::time::Instant;

#[derive(Clone, Debug, Serialize)]
pub struct Record {
    pub provider: String,
    pub phase: String,
    pub result: String,
    pub reason: Option<String>,
    pub duration_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub argv: Option<crate::collect::ArgvCost>,
    #[serde(skip)]
    pub(crate) started: Option<Instant>,
}

impl Record {
    pub fn new(
        provider: &str,
        phase: &str,
        result: &str,
        reason: Option<&str>,
        started: Option<Instant>,
    ) -> Self {
        Self {
            provider: provider.chars().take(64).collect(),
            phase: phase.into(),
            result: result.into(),
            reason: reason.map(|value| value.chars().take(128).collect()),
            duration_ms: started.map(|start| start.elapsed().as_secs_f64() * 1000.0),
            argv: None,
            started,
        }
    }
    pub fn issue(&self) -> bool {
        self.result == "failed" || self.reason.as_deref() == Some("invalid_settings_or_dependency")
    }
    pub fn line(&self) -> String {
        format!(
            "{} {} {} {} {}",
            self.provider,
            self.phase,
            self.result,
            self.duration_ms
                .map_or("-".into(), |ms| format!("{ms:.3}ms")),
            self.reason.as_deref().unwrap_or("")
        )
    }
}
