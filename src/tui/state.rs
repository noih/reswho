use crate::{
    model::{Node, Snapshot},
    tree,
};
use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    time::Instant,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Sort {
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

pub(super) struct Row {
    pub path: Vec<usize>,
    pub key: String,
    pub depth: usize,
    pub has_children: bool,
    pub unreadable: usize,
}

pub(super) struct App {
    pub snapshot: Option<Snapshot>,
    pub rows: Vec<Row>,
    pub selected: usize,
    pub expanded: HashMap<String, bool>,
    pub query: String,
    pub editing: bool,
    pub sort: Sort,
    pub active_only: bool,
    pub path_scroll: u16,
    pub help: bool,
    pub error: Option<String>,
    pub updated: Option<Instant>,
    pub collecting: bool,
    pub loading_frame: usize,
    pub paused: bool,
    scope: String,
    pinned_scope: Option<String>,
    initial_depth: usize,
    generation: u64,
}

fn identity(node: &Node, generation: u64) -> String {
    match &node.process {
        Some(process) => match process.started_unix_us {
            Some(start) => format!("{}@{start}", node.reference),
            None => format!("{}@unverified-{generation}", node.reference),
        },
        None => node.reference.clone(),
    }
}

fn searchable(node: &Node) -> String {
    let mut values = vec![node.name.clone(), node.reference.clone()];
    values.extend(node.evidence.clone());
    if let Some(process) = &node.process {
        for path in [&process.cwd, &process.executable].into_iter().flatten() {
            values.push(path.to_string_lossy().into_owned());
        }
    }
    if let Some(container) = &node.container {
        values.push(container.id.clone());
        if let Some(project) = &container.project {
            values.push(project.name.clone());
            if let Some(path) = &project.path {
                values.push(path.to_string_lossy().into_owned());
            }
        }
        for mount in &container.mounts {
            values.push(mount.source.to_string_lossy().into_owned());
            values.push(mount.destination.to_string_lossy().into_owned());
        }
        for path in &container.candidates {
            values.push(path.to_string_lossy().into_owned());
        }
    }
    values.join("\n").to_lowercase()
}

fn node_at<'a>(root: &'a Node, path: &[usize]) -> &'a Node {
    path.iter().fold(root, |node, index| &node.children[*index])
}

impl App {
    pub fn new(scope: String, initial_depth: usize) -> Self {
        Self {
            snapshot: None,
            rows: Vec::new(),
            selected: 0,
            expanded: HashMap::new(),
            query: String::new(),
            editing: false,
            sort: Sort::Memory,
            active_only: false,
            path_scroll: 0,
            help: false,
            error: None,
            updated: None,
            collecting: false,
            loading_frame: 0,
            paused: false,
            scope,
            pinned_scope: None,
            initial_depth,
            generation: 0,
        }
    }

    pub fn node(&self, row: usize) -> Option<&Node> {
        let row = self.rows.get(row)?;
        if row.unreadable > 0 {
            return None;
        }
        let path = &row.path;
        Some(node_at(&self.snapshot.as_ref()?.root, path))
    }

    fn selection_chain(&self) -> Vec<String> {
        let Some(row) = self.rows.get(self.selected) else {
            return Vec::new();
        };
        let Some(snapshot) = &self.snapshot else {
            return Vec::new();
        };
        std::iter::once(row.key.clone())
            .chain((0..=row.path.len()).rev().map(|length| {
                identity(
                    node_at(&snapshot.root, &row.path[..length]),
                    self.generation,
                )
            }))
            .collect()
    }

    pub fn update(&mut self, result: Result<Snapshot, String>) {
        self.collecting = false;
        let mut snapshot = match result {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        let root = match tree::select(&snapshot.root, &self.scope) {
            Ok(root) => root,
            Err(error) => {
                self.error = Some(error.to_string());
                return;
            }
        };
        let next_generation = self.generation + 1;
        let key = identity(&root, next_generation);
        if self.scope.starts_with("pid:")
            && self.pinned_scope.as_ref().is_some_and(|old| old != &key)
        {
            self.error = Some(
                "Selected PID identity changed or cannot be verified; restart with a current ref"
                    .into(),
            );
            return;
        }
        let chain = self.selection_chain();
        self.pinned_scope = Some(key);
        snapshot.root = root;
        snapshot.scope = self.scope.clone();
        self.snapshot = Some(snapshot);
        self.generation = next_generation;
        self.error = None;
        self.updated = Some(Instant::now());
        let mut current = HashSet::new();
        if let Some(snapshot) = &self.snapshot {
            let mut pending = vec![&snapshot.root];
            while let Some(node) = pending.pop() {
                current.insert(identity(node, self.generation));
                pending.extend(&node.children);
            }
        }
        self.expanded.retain(|key, _| current.contains(key));
        self.rebuild_with_chain(chain);
    }

    pub fn rebuild(&mut self) {
        self.rebuild_with_chain(self.selection_chain());
    }

    fn rebuild_with_chain(&mut self, chain: Vec<String>) {
        self.rows = self
            .snapshot
            .as_ref()
            .map(|snapshot| self.project(&snapshot.root, Vec::new(), false, 0))
            .unwrap_or_default();
        self.selected = chain
            .iter()
            .find_map(|key| self.rows.iter().position(|row| &row.key == key))
            .unwrap_or(0);
        if self.rows.get(self.selected).map(|row| &row.key) != chain.first() {
            self.path_scroll = 0;
        }
    }

    fn transparent(&self, node: &Node, path: &[usize]) -> bool {
        if path.is_empty() {
            return self.scope == "group:all";
        }
        let parent = node_at(
            &self.snapshot.as_ref().expect("projection snapshot").root,
            &path[..path.len() - 1],
        );
        matches!(node.grouping, Some(crate::model::Grouping::System))
            && matches!(parent.grouping, Some(crate::model::Grouping::Category))
            && parent.children.len() == 1
    }

    fn project(
        &self,
        node: &Node,
        path: Vec<usize>,
        inherited_match: bool,
        depth: usize,
    ) -> Vec<Row> {
        let transparent = self.transparent(node, &path);
        if self.active_only
            && node
                .container
                .as_ref()
                .is_some_and(|container| container.is_inactive())
            && !path.is_empty()
        {
            return Vec::new();
        }
        let matches = self.query.is_empty()
            || inherited_match
            || (!transparent && searchable(node).contains(&self.query.to_lowercase()));
        let child_depth = depth + usize::from(!transparent);
        let mut indices: Vec<_> = (0..node.children.len()).collect();
        indices.sort_by(|a, b| {
            let left = &node.children[*a];
            let right = &node.children[*b];
            if self.sort == Sort::Name {
                return natural_cmp(&left.name, &right.name)
                    .then(left.reference.cmp(&right.reference));
            }
            match (self.sort.value(left), self.sort.value(right)) {
                (Some(a), Some(b)) => b.total_cmp(&a),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                _ => Ordering::Equal,
            }
            .then(left.reference.cmp(&right.reference))
        });
        let mut children = Vec::new();
        let mut unreadable = 0;
        for index in indices {
            let child = &node.children[index];
            if child
                .process
                .as_ref()
                .is_some_and(|process| process.is_unreadable())
                && child.children.is_empty()
            {
                unreadable += 1;
                continue;
            }
            let mut child_path = path.clone();
            child_path.push(index);
            children.extend(self.project(
                &node.children[index],
                child_path,
                matches && !self.query.is_empty(),
                child_depth,
            ));
        }
        if unreadable > 0 && (matches || "unreadable".contains(&self.query.to_lowercase())) {
            children.push(Row {
                key: format!("{}:unreadable", identity(node, self.generation)),
                path: path.clone(),
                depth: child_depth,
                has_children: false,
                unreadable,
            });
        }
        if !path.is_empty()
            && self.active_only
            && node.container_totals.as_ref().is_some_and(|totals| {
                totals.containers > 0 && totals.containers == totals.inactive_containers
            })
        {
            return Vec::new();
        }
        if !matches && children.is_empty() {
            return Vec::new();
        }
        if transparent {
            return children;
        }
        let key = identity(node, self.generation);
        let open = !self.query.is_empty()
            || *self
                .expanded
                .get(&key)
                .unwrap_or(&(depth + usize::from(self.scope == "group:all") < self.initial_depth));
        let mut rows = vec![Row {
            path,
            key,
            depth,
            has_children: !children.is_empty(),
            unreadable: 0,
        }];
        if open {
            rows.extend(children);
        }
        rows
    }

    pub fn is_open(&self, row: &Row) -> bool {
        !self.query.is_empty()
            || *self.expanded.get(&row.key).unwrap_or(
                &(row.depth + usize::from(self.scope == "group:all") < self.initial_depth),
            )
    }

    pub fn move_selection(&mut self, delta: isize) {
        self.selected = self
            .selected
            .saturating_add_signed(delta)
            .min(self.rows.len().saturating_sub(1));
        self.path_scroll = 0;
    }

    pub fn expand(&mut self) {
        if let Some(row) = self.rows.get(self.selected)
            && row.has_children
        {
            if self.is_open(row) {
                self.move_selection(1);
            } else {
                self.expanded.insert(row.key.clone(), true);
                self.rebuild();
            }
        }
    }

    pub fn collapse(&mut self) {
        if let Some(row) = self.rows.get(self.selected) {
            if self.query.is_empty() && row.has_children && self.is_open(row) {
                self.expanded.insert(row.key.clone(), false);
                self.rebuild();
            } else if row.unreadable > 0 || !row.path.is_empty() {
                let length = row.path.len() - usize::from(row.unreadable == 0);
                if let Some(index) = (0..=length).rev().find_map(|length| {
                    self.rows.iter().position(|candidate| {
                        candidate.unreadable == 0 && candidate.path == row.path[..length]
                    })
                }) {
                    self.selected = index;
                    self.path_scroll = 0;
                }
            }
        }
    }

    pub fn toggle(&mut self) {
        if self
            .rows
            .get(self.selected)
            .is_some_and(|row| self.is_open(row))
        {
            self.collapse();
        } else {
            self.expand();
        }
    }
}
