use crate::{
    model::{Node, Snapshot},
    sort::Sort,
    tree,
};
use std::collections::{HashMap, HashSet};

pub(super) struct Row {
    pub path: Vec<usize>,
    pub key: String,
    pub depth: usize,
    pub has_children: bool,
    pub unreadable: usize,
}

pub(super) enum ManualRefresh {
    Refresh,
    Retry,
}

pub(super) struct App {
    pub settings: Option<super::settings::Popup>,
    pub snapshot: Option<Snapshot>,
    pub extensions: Vec<crate::extensions::report::Record>,
    pub inventory: Vec<crate::extensions::state::Listing>,
    pub extension_registry: Option<std::sync::Arc<crate::extensions::state::State>>,
    pub containers: bool,
    pub extension_selected: Option<String>,
    pub extension_details: bool,
    pub extensions_page: bool,
    pub extensions_scroll: u16,
    pub extension_details_scroll: u16,
    pub priority_draft: Option<Vec<String>>,
    pub priority_error: Option<String>,
    pub rows: Vec<Row>,
    pub selected: usize,
    pub expanded: HashMap<String, bool>,
    pub query: String,
    pub editing: bool,
    pub sort: Sort,
    pub active_only: bool,
    pub path_scroll: u16,
    pub help: bool,
    pub details: Option<super::details::Details>,
    pub error: Option<String>,
    pub collecting: bool,
    pub manual_refresh: Option<ManualRefresh>,
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
            settings: None,
            snapshot: None,
            extensions: vec![],
            inventory: vec![],
            extension_registry: None,
            containers: false,
            extension_selected: None,
            extension_details: false,
            extensions_page: false,
            extensions_scroll: 0,
            extension_details_scroll: 0,
            priority_draft: None,
            priority_error: None,
            rows: Vec::new(),
            selected: 0,
            expanded: HashMap::new(),
            query: String::new(),
            editing: false,
            sort: Sort::Memory,
            active_only: false,
            path_scroll: 0,
            help: false,
            details: None,
            error: None,
            collecting: false,
            manual_refresh: None,
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

    pub fn open_details(&mut self) {
        let Some(node) = self.node(self.selected) else {
            return;
        };
        if node.process.is_none() && node.resource.is_none() {
            return;
        }
        if let Some(snapshot) = &self.snapshot
            && let Ok(node) = tree::select(&snapshot.root, &node.reference)
        {
            let mut node = node;
            crate::model::project_node(&mut node, None);
            self.details = Some(super::details::Details::new(node));
        }
    }

    fn refresh_failed(&self) -> bool {
        self.error.is_some()
            || self
                .details
                .as_ref()
                .is_some_and(|details| details.retryable_error())
    }

    pub fn refresh_hint(&self) -> &'static str {
        if let Some(request) = &self.manual_refresh {
            return match request {
                ManualRefresh::Refresh => "refreshing",
                ManualRefresh::Retry => "retrying",
            };
        }
        if self
            .details
            .as_ref()
            .is_some_and(|details| details.query_pid().is_none())
        {
            return "";
        }
        if self.refresh_failed() {
            "r retry"
        } else {
            "r refresh"
        }
    }

    pub fn request_refresh(&mut self) {
        if self
            .details
            .as_ref()
            .is_some_and(|details| details.query_pid().is_none())
        {
            return;
        }
        if self.manual_refresh.is_none() {
            self.manual_refresh = Some(if self.refresh_failed() {
                ManualRefresh::Retry
            } else {
                ManualRefresh::Refresh
            });
        }
        if !self.collecting
            && let Some(details) = &mut self.details
        {
            details.refresh();
        }
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
        self.manual_refresh = None;
        if let Some(details) = &mut self.details {
            details.update(&result);
        }
        let mut snapshot = match result {
            Ok(snapshot) => {
                self.extensions = snapshot.extensions.clone();
                snapshot
            }
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
        snapshot.project(None);
        snapshot.scope = self.scope.clone();
        self.snapshot = Some(snapshot);
        self.generation = next_generation;
        self.error = None;
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
        indices.sort_by(|a, b| self.sort.compare(&node.children[*a], &node.children[*b]));
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
