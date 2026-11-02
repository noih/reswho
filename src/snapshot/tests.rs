use super::*;
use crate::{
    extensions::{
        self, Capability, Contribution, Descriptor, Extension, ProcessIdentity, Request,
        report::Record,
    },
    model::{DetailField, Process},
};

struct Details(&'static str, bool);
impl Extension for Details {
    fn descriptor(&self) -> Descriptor {
        Descriptor {
            id: self.0.into(),
            version: "1.0.0".into(),
            protocol_version: 1,
            source: "builtin",
            platforms: vec!["macos".into()],
            capabilities: vec![Capability::Details],
            enabled: true,
            availability: "unknown",
        }
    }
    fn execute(&self, _: &extensions::Context<'_>, request: Request<'_>) -> Result<Contribution> {
        if self.1 {
            bail!("SECRET_SENTINEL");
        }
        let Request::Details { process, .. } = request else {
            bail!("expected details")
        };
        Ok(Contribution::ParsedDetails {
            identity: ProcessIdentity::observed(process).unwrap(),
            fields: vec![DetailField {
                key: format!("extensions.{}.value", self.0),
                label: "Value".into(),
                value: serde_json::json!(1),
                sensitive: false,
            }],
        })
    }
}

#[test]
fn final_identity_rejection_invalidates_only_contributing_details_and_suppresses_retry() {
    let settings = extensions::default_settings().unwrap();
    let cancel = Cancellation::default();
    let context = extensions::Context::new(&settings, &cancel);
    let mut process = Process {
        pid: 42,
        started_unix_us: Some(123),
        ..Default::default()
    };
    let good = Details("good", false);
    let bad = Details("bad", true);
    assert_eq!(
        extensions::external::attach_details(&[&good, &bad], &context, &mut process).len(),
        1
    );
    context.report.lock().unwrap().extend([
        Record::new(
            "unused",
            Some("1.0.0"),
            "details",
            "never_invoked",
            Some("not_requested"),
            None,
        ),
        Record::new(
            "healthy",
            Some("1.0.0"),
            "classify",
            "success",
            None,
            Some(Instant::now()),
        ),
    ]);
    validate_external_details(&context, &mut process, |observed| {
        assert_eq!(observed.pid, 42);
        assert_eq!(observed.started_unix_us, Some(123));
        false
    });
    assert!(process.extension_details.is_empty());
    assert!(
        process
            .unavailable
            .iter()
            .any(|diagnostic| diagnostic.code == "identity_changed")
    );
    let records = context.report.lock().unwrap().clone();
    let good_record = records
        .iter()
        .find(|record| record.provider == "good")
        .unwrap();
    assert_eq!(good_record.result, "failed");
    assert_eq!(good_record.reason.as_deref(), Some("identity_changed"));
    assert!(good_record.duration_ms.is_some());
    assert_eq!(
        records
            .iter()
            .find(|record| record.provider == "bad")
            .unwrap()
            .reason
            .as_deref(),
        Some("provider_failed")
    );
    assert_eq!(
        records
            .iter()
            .find(|record| record.provider == "unused")
            .unwrap()
            .result,
        "never_invoked"
    );
    assert_eq!(
        records
            .iter()
            .find(|record| record.provider == "healthy")
            .unwrap()
            .result,
        "success"
    );
    assert!(!serde_json::to_string(&records).unwrap().contains("SECRET"));
    assert!(
        extensions::dispatch(
            &good,
            &context,
            Request::Details {
                process: &process,
                pointer_width: 8
            }
        )
        .is_err()
    );
    assert_eq!(
        context.report.lock().unwrap().last().unwrap().result,
        "skipped"
    );
    let schema: serde_json::Value = serde_json::from_str(crate::output::JSON_SCHEMA).unwrap();
    let mut snapshot = Snapshot {
        extensions: records,
        schema: crate::model::JSON_SCHEMA_VERSION,
        captured_unix_ms: 0,
        sample_ms: 250,
        collection_ms: 1,
        system: Default::default(),
        scope: "group:all".into(),
        warnings: vec![],
        root: crate::tree::build(vec![]).unwrap(),
    };
    assert!(
        jsonschema::validator_for(&schema)
            .unwrap()
            .is_valid(&serde_json::to_value(&snapshot).unwrap())
    );
    snapshot.extensions = context.report.lock().unwrap().clone();
    assert!(
        jsonschema::validator_for(&schema)
            .unwrap()
            .is_valid(&serde_json::to_value(&snapshot).unwrap())
    );
}
