use super::*;
use crate::{
    extensions::{ClassificationEvidence, tags::Operation},
    model::Process,
};

struct Arguments(Vec<String>);
impl Argv for Arguments {
    fn read(&mut self, _: &Process) -> Option<Vec<String>> {
        Some(self.0.clone())
    }
}

#[test]
fn resume_arguments_never_claim_current_session_or_use_work_ownership() {
    let process = Process {
        pid: 42,
        uid: Some(unsafe { libc::geteuid() }),
        started_unix_us: Some(100),
        executable: Some("/fixture/codex".into()),
        cwd: Some("/same/project".into()),
        ..Default::default()
    };
    for args in [
        vec!["codex", "resume", "019aa1bb-2222-7333-8444-555566667777"],
        vec!["codex", "app-server", "--managed-daemon"],
    ] {
        let proposals = proposals(
            &Facts::new(std::slice::from_ref(&process)),
            &mut Arguments(args.into_iter().map(Into::into).collect()),
        );
        assert_eq!(proposals.len(), 3);
        for proposal in proposals {
            let ClassificationEvidence::Tags { proposal } = proposal.evidence else {
                panic!()
            };
            let Operation::Add { tag } = proposal.operation else {
                panic!()
            };
            assert_ne!(tag.dimension, "work");
            assert_ne!(tag.hint.as_ref().unwrap().kind, "session");
            assert_ne!(tag.hint.as_ref().unwrap().kind, "session_log");
            assert!(tag.assignment.is_none() && tag.projection.is_none());
        }
    }
}

#[test]
fn role_producer_and_untitled_log_fallback_obey_extension_disable() {
    use crate::extensions::{self, Builtin};
    let process = Process {
        pid: 42,
        uid: Some(unsafe { libc::geteuid() }),
        started_unix_us: Some(100),
        executable: Some("/fixture/codex".into()),
        ..Default::default()
    };
    let facts = Facts::new(std::slice::from_ref(&process));
    let proposals = proposals(
        &facts,
        &mut Arguments(vec!["codex".into(), "app-server".into()]),
    );
    let fixture = extensions::tests::Fixture::new("codex", Contribution::Classify { proposals });
    let mut settings = extensions::default_settings().unwrap();
    let cancel = crate::snapshot::Cancellation::default();
    let (mut reports, diagnostics) =
        extensions::tags::classify(&[&fixture], &Context::new(&settings, &cancel), &facts);
    assert!(diagnostics.is_empty());
    let mut node = crate::model::Node::group(
        "pid:42".into(),
        "codex".into(),
        vec![],
        crate::model::Grouping::Bin,
    );
    node.process = Some(Box::new(process));
    node.process.as_mut().unwrap().tag_report = reports.remove(&42);
    extensions::host::hints::summarize(&mut node);
    for untitled in [false, true] {
        if untitled {
            node.hints.push(crate::model::SummaryHint {
                source: "codex".into(),
                basis: "open_session_log".into(),
                kind: "session_log".into(),
                value: "019aa1bb-2222-7333-8444-555566667777".into(),
                title: None,
                path: None,
                processes: 1,
            });
        }
        extensions::summaries::attach(
            &mut node,
            &[&Builtin::Codex],
            &Context::new(&settings, &cancel),
        );
        assert_eq!(node.summary.as_ref().unwrap().value, "Codex app server");
        assert_eq!(node.summary.as_ref().unwrap().label, "Tool role");
    }
    settings.set_builtin_enabled("codex", false).unwrap();
    extensions::summaries::attach(
        &mut node,
        &[&Builtin::Codex],
        &Context::new(&settings, &cancel),
    );
    assert!(node.summary.is_none());
}
