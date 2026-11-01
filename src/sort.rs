use crate::model::Node;
use std::cmp::Ordering;

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Sort {
    Name,
    Memory,
    Cpu,
    Uptime,
}

impl Sort {
    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "NAME",
            Self::Memory => "RAM",
            Self::Cpu => "CPU",
            Self::Uptime => "UPTIME",
        }
    }
    pub fn compare(self, left: &Node, right: &Node) -> Ordering {
        if self == Self::Name {
            return natural_cmp(&left.name, &right.name).then(left.reference.cmp(&right.reference));
        }
        match (self.value(left), self.value(right)) {
            (Some(a), Some(b)) => b.total_cmp(&a),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            _ => Ordering::Equal,
        }
        .then(left.reference.cmp(&right.reference))
    }

    pub fn apply(self, node: &mut Node) {
        node.children
            .sort_by(|left, right| self.compare(left, right));
        for child in &mut node.children {
            self.apply(child);
        }
    }

    fn value(self, node: &Node) -> Option<f64> {
        if let Some(process) = &node.process {
            return match self {
                Self::Name => None,
                Self::Memory => process.resident_bytes.map(|value| value as f64),
                Self::Cpu => process.cpu_percent,
                Self::Uptime => process.uptime_seconds.map(|value| value as f64),
            };
        }
        if let Some(container) = &node.container {
            if container.is_inactive() {
                return None;
            }
            return match self {
                Self::Name => None,
                Self::Memory => container.memory_bytes.map(|value| value as f64),
                Self::Cpu => container.cpu_percent,
                Self::Uptime => container.uptime_seconds.map(|value| value as f64),
            };
        }
        if let Some(totals) = &node.container_totals {
            let count = totals.containers - totals.inactive_containers;
            return match self {
                Self::Name => None,
                Self::Memory => {
                    (count > totals.memory_unknown).then_some(totals.known_memory_bytes as f64)
                }
                Self::Cpu => (count > totals.cpu_unknown).then_some(totals.known_cpu_percent),
                Self::Uptime => totals.oldest_uptime_seconds.map(|value| value as f64),
            };
        }
        match self {
            Self::Name => None,
            Self::Memory => (node.totals.processes > node.totals.memory_unknown)
                .then_some(node.totals.resident_bytes as f64),
            Self::Cpu => {
                (node.totals.processes > node.totals.cpu_unknown).then_some(node.totals.cpu_percent)
            }
            Self::Uptime => node.totals.oldest_uptime_seconds.map(|value| value as f64),
        }
    }
}

fn natural_cmp(left: &str, right: &str) -> Ordering {
    let left = left.to_lowercase();
    let right = right.to_lowercase();
    let (left, right) = (left.as_bytes(), right.as_bytes());
    let (mut a, mut b) = (0, 0);
    while a < left.len() && b < right.len() {
        let order = if left[a].is_ascii_digit() && right[b].is_ascii_digit() {
            let (start_a, start_b) = (a, b);
            while a < left.len() && left[a].is_ascii_digit() {
                a += 1;
            }
            while b < right.len() && right[b].is_ascii_digit() {
                b += 1;
            }
            let digits_a = &left[start_a..a];
            let digits_b = &right[start_b..b];
            let number_a = &digits_a[digits_a
                .iter()
                .position(|digit| *digit != b'0')
                .unwrap_or(digits_a.len())..];
            let number_b = &digits_b[digits_b
                .iter()
                .position(|digit| *digit != b'0')
                .unwrap_or(digits_b.len())..];
            number_a
                .len()
                .cmp(&number_b.len())
                .then(number_a.cmp(number_b))
                .then(digits_a.len().cmp(&digits_b.len()))
        } else {
            let order = left[a].cmp(&right[b]);
            a += 1;
            b += 1;
            order
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    (left.len() - a).cmp(&(right.len() - b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        containers::{Container, ContainerTotals},
        model::{Grouping, Process},
        output, tree,
    };

    fn process(pid: u32, memory: Option<u64>, cpu: Option<f64>, age: Option<u64>) -> Node {
        let process = Process {
            pid,
            resident_bytes: memory,
            cpu_percent: cpu,
            uptime_seconds: age,
            ..Process::default()
        };
        let mut node = Node::group(
            format!("pid:{pid}"),
            format!("node{pid}"),
            vec![],
            Grouping::Bin,
        );
        node.kind = "process";
        node.grouping = None;
        node.totals.add_process(&process);
        node.process = Some(process);
        node
    }

    fn group() -> Node {
        Node::group("group:test".into(), "Example".into(), vec![], Grouping::App)
    }

    #[test]
    fn sorting_uses_displayed_own_metrics_preserving_subtrees_totals_and_refs() {
        let mut parent = process(10, Some(1), Some(2.0), Some(3));
        let child = process(11, Some(1000), Some(100.0), Some(1));
        parent.totals.add(&child.totals);
        parent.children.push(child);
        let mut root = group();
        root.children = vec![
            parent,
            process(2, Some(10), Some(1.0), Some(4)),
            process(3, None, None, None),
            process(4, Some(0), Some(0.0), Some(0)),
        ];
        for child in &root.children {
            root.totals.add(&child.totals);
        }
        let before = serde_json::to_value(&root.totals).unwrap();
        for (sort, expected) in [
            (Sort::Memory, [2, 10, 4, 3]),
            (Sort::Cpu, [10, 2, 4, 3]),
            (Sort::Uptime, [2, 10, 4, 3]),
            (Sort::Name, [2, 3, 4, 10]),
        ] {
            sort.apply(&mut root);
            assert_eq!(
                root.children
                    .iter()
                    .map(|node| node.process.as_ref().unwrap().pid)
                    .collect::<Vec<_>>(),
                expected
            );
            assert_eq!(serde_json::to_value(&root.totals).unwrap(), before);
            let selected = tree::select(&root, "pid:10").unwrap();
            assert_eq!(selected.children[0].reference, "pid:11");
            assert_eq!(selected.totals.resident_bytes, 1001);
        }
        tree::limit_depth(&mut root, 1);
        assert_eq!(root.children[3].omitted_children, 1);
        assert_eq!(root.children[3].totals.resident_bytes, 1001);
    }

    #[test]
    fn container_and_group_sort_uses_own_scope_and_places_inactive_and_unknown_last() {
        let mut runtime = group();
        runtime.container_totals = Some(ContainerTotals {
            containers: 2,
            active_containers: 2,
            known_memory_bytes: 20,
            known_cpu_percent: 4.0,
            memory_unknown: 1,
            cpu_unknown: 1,
            oldest_uptime_seconds: Some(30),
            ..ContainerTotals::default()
        });
        let mut running = group();
        running.container = Some(Container {
            state: Some("running".into()),
            memory_bytes: Some(10),
            cpu_percent: Some(2.0),
            uptime_seconds: Some(10),
            ..Container::default()
        });
        let mut stopped = running.clone();
        stopped.container.as_mut().unwrap().state = Some("exited".into());
        stopped.container.as_mut().unwrap().memory_bytes = Some(9999);
        for sort in [Sort::Memory, Sort::Cpu, Sort::Uptime] {
            assert_eq!(sort.compare(&runtime, &running), Ordering::Less);
            assert_eq!(sort.compare(&running, &stopped), Ordering::Less);
            assert_eq!(sort.compare(&running, &group()), Ordering::Less);
        }
        runtime.container_totals.as_mut().unwrap().memory_unknown = 2;
        assert_eq!(Sort::Memory.compare(&runtime, &running), Ordering::Greater);
    }

    #[test]
    fn markdown_and_json_follow_the_same_natural_order() {
        let mut root = group();
        root.children = vec![
            process(10, Some(1), None, None),
            process(2, Some(1), None, None),
            process(1, Some(1), None, None),
        ];
        Sort::Name.apply(&mut root);
        let snapshot = crate::model::Snapshot {
            schema: crate::model::JSON_SCHEMA_VERSION,
            captured_unix_ms: 0,
            sample_ms: 250,
            collection_ms: 300,
            system: Default::default(),
            scope: root.reference.clone(),
            warnings: vec![],
            root,
        };
        let md = output::markdown(&snapshot);
        assert!(md.find("node1 pid:1").unwrap() < md.find("node2 pid:2").unwrap());
        assert!(md.find("node2 pid:2").unwrap() < md.find("node10 pid:10").unwrap());
        let json = serde_json::to_value(snapshot).unwrap();
        assert_eq!(json["root"]["children"][0]["reference"], "pid:1");
        assert_eq!(json["root"]["children"][2]["reference"], "pid:10");
    }
}
