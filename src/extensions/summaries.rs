use super::{Context, Extension};
use crate::model::{Diagnostic, Node, SummaryHint};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    pub label: String,
    pub value: String,
    pub priority: u16,
    pub inherited: bool,
    pub evidence: Vec<SummaryHint>,
}

impl Summary {
    pub fn new(label: &str, value: &str, priority: u16, evidence: Vec<SummaryHint>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            priority,
            inherited: false,
            evidence,
        }
    }
}

pub struct SummaryContext<'a> {
    pub node: &'a Node,
    pub ancestors: &'a [&'a Node],
    pub source: &'a str,
}

impl SummaryContext<'_> {
    pub fn hints(&self) -> impl Iterator<Item = &SummaryHint> {
        self.node
            .hints
            .iter()
            .filter(|hint| hint.source == self.source)
    }
}

fn candidate(
    extension: &dyn Extension,
    context: &Context<'_>,
    node: &Node,
    ancestors: &[&Node],
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<(usize, Summary)> {
    let mut provider = None;
    let mut invoked = false;
    let started = Instant::now();
    let result = super::boundary(|| {
        let descriptor = extension.descriptor();
        provider = Some(descriptor.id.clone());
        context.check()?;
        if context
            .failed
            .lock()
            .expect("round failure lock")
            .contains(&descriptor.id)
        {
            anyhow::bail!("extension failed earlier in this round");
        }
        if !descriptor.enabled
            || context.settings.is_disabled(&descriptor.id)
            || !descriptor
                .capabilities
                .contains(&super::Capability::Classify)
        {
            anyhow::bail!("extension capability is disabled or unsupported");
        }
        invoked = true;
        let summary = extension.summary(&SummaryContext {
            node,
            ancestors,
            source: &descriptor.id,
        });
        context.check()?;
        if let Some(summary) = &summary {
            let valid = |text: &str| {
                !text.trim().is_empty() && text.len() <= 4096
                && !text.chars().any(|c| c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{2028}' | '\u{2029}'))
            };
            if !valid(&summary.label)
                || !valid(&summary.value)
                || summary.evidence.is_empty()
                || !summary
                    .evidence
                    .iter()
                    .all(|hint| hint.source == descriptor.id && node.hints.contains(hint))
            {
                anyhow::bail!("invalid summary contribution");
            }
        }
        Ok(summary.map(|summary| {
            let order = context
                .settings
                .source_priority
                .iter()
                .position(|id| id == &descriptor.id)
                .unwrap_or(context.settings.source_priority.len());
            (order, summary)
        }))
    });
    let reason = if context.cancel.check().is_err() {
        "canceled"
    } else if Instant::now() >= context.deadline {
        "deadline"
    } else if provider.is_none() {
        "metadata_failed"
    } else if invoked {
        "provider_failed"
    } else {
        "not_enabled_or_prior_failure"
    };
    let record = super::report::Record::new(
        provider.as_deref().unwrap_or("unidentified"),
        "classify",
        if result.is_ok() {
            "success"
        } else if invoked || provider.is_none() {
            "failed"
        } else {
            "skipped"
        },
        result.as_ref().err().map(|_| reason),
        (invoked || provider.is_none()).then_some(started),
    );
    {
        let mut records = context.report.lock().expect("report lock");
        if let Some(old) = records
            .iter_mut()
            .find(|old| old.provider == record.provider && old.phase == "classify")
        {
            if invoked || provider.is_none() {
                if let (Some(total), Some(elapsed)) = (&mut old.duration_ms, record.duration_ms) {
                    *total += elapsed;
                }
                if record.result == "failed" {
                    old.result = record.result;
                    old.reason = record.reason;
                }
            }
        } else if records.len() < 2048 {
            records.push(record);
        }
    }
    let result = context.finish_phase(provider.as_deref(), "classify", Duration::ZERO, result);
    if result.is_err() && (invoked || provider.is_none()) {
        diagnostics.push(super::invalid(
            provider.as_deref().unwrap_or("unidentified"),
            "classify",
        ));
    }
    result.ok().flatten()
}

pub(crate) fn attach(root: &mut Node, providers: &[&dyn Extension], context: &Context<'_>) {
    fn visit<'a>(
        node: &'a Node,
        ancestors: &mut Vec<&'a Node>,
        providers: &[&dyn Extension],
        context: &Context<'_>,
        summaries: &mut Vec<Option<Summary>>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let summary = if node.process.is_some() {
            providers
                .iter()
                .filter_map(|provider| candidate(*provider, context, node, ancestors, diagnostics))
                .min_by_key(|(order, summary)| (*order, summary.priority))
                .map(|(_, summary)| summary)
        } else {
            None
        };
        summaries.push(summary);
        ancestors.push(node);
        for child in &node.children {
            visit(child, ancestors, providers, context, summaries, diagnostics);
        }
        ancestors.pop();
    }
    let mut summaries = Vec::new();
    let mut diagnostics = Vec::new();
    visit(
        root,
        &mut Vec::new(),
        providers,
        context,
        &mut summaries,
        &mut diagnostics,
    );
    fn install(node: &mut Node, summaries: &mut impl Iterator<Item = Option<Summary>>) {
        node.summary = summaries.next().flatten();
        for child in &mut node.children {
            install(child, summaries);
        }
    }
    install(root, &mut summaries.into_iter());
    for diagnostic in diagnostics {
        if !root.diagnostics.contains(&diagnostic) {
            root.diagnostics.push(diagnostic);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::{self, Capability, Contribution, Descriptor, Request};
    use crate::model::Process;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Provider {
        id: &'static str,
        panic: bool,
        summaries: AtomicUsize,
        executions: AtomicUsize,
    }

    impl Extension for Provider {
        fn descriptor(&self) -> Descriptor {
            Descriptor {
                id: self.id.into(),
                capabilities: vec![Capability::Classify, Capability::Details],
                enabled: true,
                availability: "available",
            }
        }
        fn execute(&self, _: &Context<'_>, request: Request<'_>) -> anyhow::Result<Contribution> {
            self.executions.fetch_add(1, Ordering::SeqCst);
            let Request::Classify(facts) = request else {
                anyhow::bail!("unexpected execution");
            };
            Ok(Contribution::Classify {
                proposals: facts
                    .processes
                    .iter()
                    .filter_map(|process| {
                        extensions::host::hints::proposal(
                            self.id,
                            extensions::host::hints::value("role", "Observed worker"),
                            "arguments",
                            None,
                            extensions::host::hints::chain(facts, process),
                        )
                    })
                    .collect(),
            })
        }
        fn summary(&self, context: &SummaryContext<'_>) -> Option<Summary> {
            self.summaries.fetch_add(1, Ordering::SeqCst);
            assert!(!self.panic, "private callback payload");
            let hint = context.hints().next()?;
            Some(Summary::new(
                "Observation",
                &hint.value,
                0,
                vec![hint.clone()],
            ))
        }
    }

    #[test]
    fn summary_panic_is_reported_once_and_blocks_later_nodes_and_capabilities() {
        let failed = Provider {
            id: "failed-summary",
            panic: true,
            summaries: AtomicUsize::new(0),
            executions: AtomicUsize::new(0),
        };
        let healthy = Provider {
            id: "healthy-summary",
            panic: false,
            summaries: AtomicUsize::new(0),
            executions: AtomicUsize::new(0),
        };
        let settings = extensions::default_settings().unwrap();
        let cancel = crate::snapshot::Cancellation::default();
        let context = Context::new(&settings, &cancel);
        let processes: Vec<_> = [81, 82]
            .into_iter()
            .map(|pid| Process {
                pid,
                ppid: Some(1),
                uid: Some(unsafe { libc::geteuid() }),
                started_unix_us: Some(u64::from(pid)),
                executable: Some("/fixture/worker".into()),
                name: "worker".into(),
                ..Default::default()
            })
            .collect();
        let tree =
            crate::tree::build_with_providers(processes.clone(), &context, &[&failed, &healthy])
                .unwrap();
        assert_eq!(tree.totals.processes, 2);
        assert_eq!(failed.summaries.load(Ordering::SeqCst), 1);
        assert_eq!(healthy.summaries.load(Ordering::SeqCst), 2);
        for pid in [81, 82] {
            let node = crate::tree::select(&tree, &format!("pid:{pid}")).unwrap();
            assert_eq!(node.summary.unwrap().evidence[0].source, "healthy-summary");
        }
        let records = context.report.lock().unwrap().clone();
        assert!(
            records
                .iter()
                .any(|record| record.provider == "failed-summary"
                    && record.phase == "classify"
                    && record.result == "failed"
                    && record.reason.as_deref() == Some("provider_failed"))
        );
        assert_eq!(tree.diagnostics.len(), 1);
        let error = extensions::dispatch(
            &failed,
            &context,
            Request::Details {
                process: &processes[0],
                pointer_width: 8,
            },
        )
        .err()
        .unwrap();
        assert_eq!(failed.executions.load(Ordering::SeqCst), 1);
        assert!(
            context
                .report
                .lock()
                .unwrap()
                .iter()
                .any(|record| record.provider == "failed-summary"
                    && record.phase == "details"
                    && record.result == "skipped")
        );
        let public = format!("{error:?} {records:?} {:?}", tree.diagnostics);
        assert!(!public.contains("private callback payload"));
    }

    #[test]
    fn summary_descriptor_panic_has_sanitized_metadata_failure() {
        struct Broken;
        impl Extension for Broken {
            fn descriptor(&self) -> Descriptor {
                panic!("private descriptor payload")
            }
            fn execute(&self, _: &Context<'_>, _: Request<'_>) -> anyhow::Result<Contribution> {
                unreachable!()
            }
        }
        let settings = extensions::default_settings().unwrap();
        let cancel = crate::snapshot::Cancellation::default();
        let context = Context::new(&settings, &cancel);
        let node = Node::group(
            "fixture".into(),
            "fixture".into(),
            vec![],
            crate::model::Grouping::Bin,
        );
        let mut diagnostics = Vec::new();
        assert!(candidate(&Broken, &context, &node, &[], &mut diagnostics).is_none());
        let records = context.report.lock().unwrap();
        assert_eq!(records[0].provider, "unidentified");
        assert_eq!(records[0].result, "failed");
        assert_eq!(records[0].reason.as_deref(), Some("metadata_failed"));
        assert_eq!(diagnostics.len(), 1);
        assert!(!format!("{records:?} {diagnostics:?}").contains("private descriptor payload"));
    }
}
