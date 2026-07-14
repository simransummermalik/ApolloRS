#![forbid(unsafe_code)]
//! Include, symbol-reference, call, and conservative control-flow graphs.

use agc_ast::SourceUnit;
use agc_ir::{Operand, ProgramIr};
use agc_symbols::SymbolTable;
use petgraph::algo::kosaraju_scc;
use petgraph::graph::{DiGraph, NodeIndex};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::Write as _;

/// Serializable directed graph with stable node and edge order.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct GraphArtifact {
    /// Node labels.
    pub nodes: Vec<String>,
    /// Directed edges as node indices.
    pub edges: Vec<(usize, usize)>,
}

impl GraphArtifact {
    /// Emits deterministic Graphviz DOT.
    pub fn to_dot(&self, name: &str) -> String {
        let mut output = format!("digraph \"{}\" {{\n", escape(name));
        for (index, node) in self.nodes.iter().enumerate() {
            let _ = writeln!(output, "  n{index} [label=\"{}\"];", escape(node));
        }
        for (from, to) in &self.edges {
            let _ = writeln!(output, "  n{from} -> n{to};");
        }
        output.push_str("}\n");
        output
    }
}

/// Builds a direct include graph from parsed units and resolved path names.
pub fn include_graph(units: &[SourceUnit]) -> GraphArtifact {
    let mut names = units
        .iter()
        .map(|unit| unit.source.relative_path.clone())
        .collect::<Vec<_>>();
    for unit in units {
        names.extend(unit.includes().map(|(_, path)| path.text.clone()));
    }
    names.sort();
    names.dedup();
    let indices = names
        .iter()
        .enumerate()
        .map(|(index, name)| (name.clone(), index))
        .collect::<BTreeMap<_, _>>();
    let mut edges = Vec::new();
    for unit in units {
        let from = indices[&unit.source.relative_path];
        for (_, path) in unit.includes() {
            if let Some(&to) = indices.get(&path.text) {
                edges.push((from, to));
            }
        }
    }
    edges.sort_unstable();
    edges.dedup();
    GraphArtifact {
        nodes: names,
        edges,
    }
}

/// Builds a symbol-level call graph from transfer-control instructions.
pub fn call_graph(ir: &ProgramIr, symbols: &SymbolTable) -> GraphArtifact {
    let mut names = symbols
        .iter()
        .map(|(name, _)| name.to_owned())
        .collect::<Vec<_>>();
    names.sort();
    let indices = names
        .iter()
        .enumerate()
        .map(|(index, name)| (name.clone(), index))
        .collect::<BTreeMap<_, _>>();
    let mut current_label = None;
    let mut edges = Vec::new();
    for record in &ir.records {
        if let Some(label) = &record.label
            && indices.contains_key(label)
        {
            current_label = Some(label.as_str());
        }
        if matches!(record.operation.as_str(), "TC" | "TCR" | "CALL" | "CCALL")
            && let (Some(from), Operand::Symbol { name: to, .. }) = (current_label, &record.operand)
            && let (Some(&from), Some(&to)) = (indices.get(from), indices.get(to))
        {
            edges.push((from, to));
        }
    }
    edges.sort_unstable();
    edges.dedup();
    GraphArtifact {
        nodes: names,
        edges,
    }
}

/// Returns recursive call groups, including self-recursion.
pub fn recursive_components(graph: &GraphArtifact) -> Vec<Vec<String>> {
    let mut petgraph = DiGraph::<(), ()>::new();
    let nodes = (0..graph.nodes.len())
        .map(|_| petgraph.add_node(()))
        .collect::<Vec<NodeIndex>>();
    for &(from, to) in &graph.edges {
        petgraph.add_edge(nodes[from], nodes[to], ());
    }
    kosaraju_scc(&petgraph)
        .into_iter()
        .filter(|component| {
            component.len() > 1
                || component
                    .first()
                    .is_some_and(|node| petgraph.find_edge(*node, *node).is_some())
        })
        .map(|component| {
            let mut labels = component
                .into_iter()
                .map(|index| graph.nodes[index.index()].clone())
                .collect::<Vec<_>>();
            labels.sort();
            labels
        })
        .collect()
}

fn escape(input: &str) -> String {
    input.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dot_escapes_historical_symbols() {
        let graph = GraphArtifact {
            nodes: vec!["A\"B".to_owned()],
            edges: vec![(0, 0)],
        };
        assert!(graph.to_dot("calls").contains("A\\\"B"));
        assert_eq!(recursive_components(&graph), vec![vec!["A\"B".to_owned()]]);
    }
}
