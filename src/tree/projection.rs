use crate::{extensions::placement::Parent, model::Node};
use std::collections::BTreeMap;

type Member = (Node, Vec<Parent>, Option<String>, bool);

pub(super) fn group(mut groups: Vec<(Node, Vec<Parent>)>) -> Vec<Node> {
    let mut finished = Vec::new();
    while !groups.is_empty() {
        let mut parents: BTreeMap<String, (Parent, Vec<Member>)> = BTreeMap::new();
        for (node, mut chain) in groups {
            if chain.is_empty() {
                finished.push(node);
                continue;
            }
            let parent = chain.remove(0);
            let child_name = parent.child_name.clone();
            let parent_merge = parent.merge;
            parents
                .entry(parent.reference.clone())
                .or_insert_with(|| (parent, vec![]))
                .1
                .push((node, chain, child_name, parent_merge));
        }
        groups = Vec::new();
        for (_, (parent, mut members)) in parents {
            let chain = members.first().expect("parent members").1.clone();
            let merging = members.iter().filter(|(_, _, _, merge)| *merge).count();
            let mut node = if let Some(index) = members.iter().position(|(node, _, _, merge)| {
                merging <= 1 && (node.reference == parent.reference || merging == 1 && *merge)
            }) {
                let mut node = members.remove(index).0;
                node.evidence.extend(
                    parent
                        .evidence
                        .iter()
                        .filter(|e| !node.evidence.contains(e))
                        .cloned()
                        .collect::<Vec<_>>(),
                );
                node.diagnostics.extend(parent.diagnostics.clone());
                node
            } else if parent.collapse_single && members.len() == 1 {
                groups.push((members.remove(0).0, chain));
                continue;
            } else {
                let mut node = Node::group(
                    if merging > 1 {
                        parent.collision_reference.unwrap_or(parent.reference)
                    } else {
                        parent.reference
                    },
                    parent.name,
                    parent.evidence,
                    parent.grouping,
                );
                node.attribution = parent.attribution;
                node.diagnostics = parent.diagnostics;
                node
            };
            for (mut child, _, child_name, _) in members {
                if let Some(name) = child_name {
                    child.name = name.clone();
                }
                node.totals.add(&child.totals);
                node.children.push(child);
            }
            groups.push((node, chain));
        }
    }
    finished
}
