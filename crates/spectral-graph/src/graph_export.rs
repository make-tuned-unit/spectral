//! Serialisable JSON export of a graph neighbourhood, for visualisation.
//!
//! Built so a caller outside Rust (a renderer, a docs page) can draw a real
//! graph without linking Spectral. Three design decisions are deliberate:
//!
//! **Explicit DTOs, not `Serialize` on the graph types.** [`Entity`],
//! [`Triple`] and [`DocumentNode`] deliberately keep no serde derives. If they
//! had them, this file format would be a mirror of internal field names, and
//! renaming a private-ish field would silently change a published format.
//! The shape below is the contract; the internal structs are free to move.
//!
//! **Visibility-scoped by construction.** [`export_neighborhood`] requires a
//! scope and drops everything inadmissible in it. An unscoped graph exporter is
//! a footgun: the natural use is publishing a picture of a real brain, and the
//! natural mistake is publishing private entities with it. Pass
//! [`Visibility::Public`] for anything that leaves the machine.
//!
//! **Stable ordering.** Nodes sort by id, edges by `(from, to, predicate)`, so
//! the same neighbourhood exports byte-identically. Imagery regenerated later
//! matches the counts published with it.
//!
//! Entity descriptions are **not** exported. They are free text written by the
//! Librarian and are a likelier carrier of incidental detail than a canonical
//! name; a renderer needs labels, not prose.

use crate::graph_store::{DocumentNode, Entity, Neighborhood, Triple};
use serde::Serialize;
use spectral_core::visibility::Visibility;

/// What a node in the exported graph is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    /// A graph entity (person, project, …).
    Entity,
    /// A source document. Terminal — never expanded further by the traversal.
    Document,
}

/// One node.
#[derive(Debug, Clone, Serialize)]
pub struct ExportNode {
    /// Hex content-address. `EntityId` for entities, blake3 doc hash for documents.
    pub id: String,
    pub kind: NodeKind,
    /// Display label: the canonical name, or the document's source string.
    pub label: String,
    /// Entity type (`person`, `project`, …). `None` for documents.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_type: Option<String>,
    pub visibility: String,
    /// Importance weight. `None` for documents, which carry none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weight: Option<f64>,
}

/// One directed edge.
#[derive(Debug, Clone, Serialize)]
pub struct ExportEdge {
    /// Hex id of the source entity.
    pub from: String,
    /// Hex id of the target entity.
    pub to: String,
    pub predicate: String,
    pub confidence: f64,
    pub weight: f64,
    pub visibility: String,
}

/// Counts and provenance for the export, so a published figure can be checked
/// against the file it came from.
#[derive(Debug, Clone, Serialize)]
pub struct ExportMeta {
    pub node_count: usize,
    pub edge_count: usize,
    pub entity_count: usize,
    pub document_count: usize,
    /// The scope this export was filtered to.
    pub visibility_scope: String,
    /// True when the traversal stopped at a budget rather than exhausting the
    /// reachable set. A renderer should say so rather than implying the picture
    /// is the whole graph.
    pub truncated: bool,
    /// Nodes and edges dropped as inadmissible in `visibility_scope`.
    pub filtered_out: usize,
}

/// A neighbourhood, ready to serialise.
#[derive(Debug, Clone, Serialize)]
pub struct GraphExport {
    pub meta: ExportMeta,
    pub nodes: Vec<ExportNode>,
    pub edges: Vec<ExportEdge>,
}

fn vis_str(v: Visibility) -> &'static str {
    match v {
        Visibility::Private => "private",
        Visibility::Team => "team",
        Visibility::Org => "org",
        Visibility::Public => "public",
    }
}

fn entity_node(e: &Entity) -> ExportNode {
    ExportNode {
        id: e.id.to_string(),
        kind: NodeKind::Entity,
        label: e.canonical.clone(),
        entity_type: Some(e.entity_type.clone()),
        visibility: vis_str(e.visibility).to_string(),
        weight: Some(e.weight),
    }
}

fn document_node(d: &DocumentNode) -> ExportNode {
    ExportNode {
        id: hex_of(&d.id),
        kind: NodeKind::Document,
        label: d.source.clone(),
        entity_type: None,
        visibility: vis_str(d.visibility).to_string(),
        weight: None,
    }
}

fn edge(t: &Triple) -> ExportEdge {
    ExportEdge {
        from: t.from.to_string(),
        to: t.to.to_string(),
        predicate: t.predicate.clone(),
        confidence: t.confidence,
        weight: t.weight,
        visibility: vis_str(t.visibility).to_string(),
    }
}

fn hex_of(bytes: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Render a traversal result as a scoped, stably-ordered graph.
///
/// Everything inadmissible in `scope` is dropped and counted in
/// [`ExportMeta::filtered_out`]. An edge is also dropped when either endpoint
/// was filtered, so the export can never contain a dangling reference to a node
/// the caller was not allowed to see — the edge's own label would otherwise
/// disclose that a hidden entity exists and what it is called.
pub fn export_neighborhood(n: &Neighborhood, scope: Visibility) -> GraphExport {
    let mut filtered_out = 0usize;

    let mut nodes: Vec<ExportNode> = Vec::new();
    let mut kept_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    for e in &n.entities {
        if e.visibility.allows(scope) {
            let node = entity_node(e);
            kept_ids.insert(node.id.clone());
            nodes.push(node);
        } else {
            filtered_out += 1;
        }
    }
    let entity_count = nodes.len();

    for d in &n.documents {
        if d.visibility.allows(scope) {
            nodes.push(document_node(d));
        } else {
            filtered_out += 1;
        }
    }
    let document_count = nodes.len() - entity_count;

    let mut edges: Vec<ExportEdge> = Vec::new();
    for t in &n.triples {
        let e = edge(t);
        // The edge itself must be admissible AND both endpoints must have
        // survived — an edge to a filtered node would leak its existence.
        if t.visibility.allows(scope) && kept_ids.contains(&e.from) && kept_ids.contains(&e.to) {
            edges.push(e);
        } else {
            filtered_out += 1;
        }
    }

    nodes.sort_by(|a, b| a.id.cmp(&b.id));
    edges.sort_by(|a, b| (&a.from, &a.to, &a.predicate).cmp(&(&b.from, &b.to, &b.predicate)));

    GraphExport {
        meta: ExportMeta {
            node_count: nodes.len(),
            edge_count: edges.len(),
            entity_count,
            document_count,
            visibility_scope: vis_str(scope).to_string(),
            truncated: n.truncated,
            filtered_out,
        },
        nodes,
        edges,
    }
}

impl GraphExport {
    /// Pretty JSON, suitable for committing next to an image.
    pub fn to_json_pretty(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use spectral_core::entity_id::{entity_id, EntityId};
    use spectral_core::identity::BrainId;

    /// Admissibility decided WITHOUT calling `Visibility::allows`, which is the
    /// predicate the exporter filters with. A test that reused `allows` here
    /// would invert with it and pass; that exact mistake was found and fixed
    /// elsewhere in this codebase, so it is not repeated.
    fn admissible_independently(label: Visibility, scope: Visibility) -> bool {
        let rank = |v: Visibility| match v {
            Visibility::Private => 0,
            Visibility::Team => 1,
            Visibility::Org => 2,
            Visibility::Public => 3,
        };
        rank(label) >= rank(scope)
    }

    /// Ids come from the real content-addressed derivation, not hand-built
    /// bytes, so the export's id strings are the ones production would emit.
    fn eid(name: &str) -> EntityId {
        entity_id("person", name)
    }

    fn entity(name: &str, vis: Visibility) -> Entity {
        let at = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        Entity {
            id: eid(name),
            entity_type: "person".into(),
            canonical: name.into(),
            visibility: vis,
            created_at: at,
            updated_at: at,
            weight: 1.0,
            description: Some("free text that must never be exported".into()),
        }
    }

    fn triple(from: &str, to: &str, vis: Visibility) -> Triple {
        Triple {
            from: eid(from),
            to: eid(to),
            predicate: "works_with".into(),
            confidence: 0.9,
            source_doc_id: None,
            source_brain_id: BrainId::from_bytes([9; 32]),
            asserted_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            visibility: vis,
            weight: 1.0,
        }
    }

    fn doc(n: u8, vis: Visibility) -> DocumentNode {
        DocumentNode {
            id: [n; 32],
            source: format!("doc-{n}.md"),
            ingested_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            visibility: vis,
        }
    }

    /// One entity per label, fully connected, so every scope has something to
    /// keep and something to drop.
    fn mixed() -> Neighborhood {
        Neighborhood {
            entities: vec![
                entity("Ada", Visibility::Private),
                entity("Bo", Visibility::Team),
                entity("Cy", Visibility::Org),
                entity("Di", Visibility::Public),
            ],
            triples: vec![
                triple("Ada", "Bo", Visibility::Private),
                triple("Bo", "Cy", Visibility::Team),
                triple("Cy", "Di", Visibility::Org),
                triple("Di", "Ada", Visibility::Public),
            ],
            documents: vec![doc(5, Visibility::Private), doc(6, Visibility::Public)],
            truncated: false,
        }
    }

    #[test]
    fn a_public_export_contains_only_public_nodes_and_edges() {
        let g = export_neighborhood(&mixed(), Visibility::Public);
        assert!(
            g.nodes.iter().all(|n| n.visibility == "public"),
            "non-public node in a public export: {:?}",
            g.nodes
        );
        assert!(g.edges.iter().all(|e| e.visibility == "public"));
        // Di (public entity) and doc-6 survive; nothing else.
        assert_eq!(g.meta.entity_count, 1);
        assert_eq!(g.meta.document_count, 1);
    }

    /// The scoping property, checked against the independent oracle for every
    /// scope rather than only the public one.
    #[test]
    fn no_scope_ever_exports_an_inadmissible_node() {
        for scope in [
            Visibility::Private,
            Visibility::Team,
            Visibility::Org,
            Visibility::Public,
        ] {
            let n = mixed();
            let g = export_neighborhood(&n, scope);
            for node in &g.nodes {
                let label = match node.visibility.as_str() {
                    "team" => Visibility::Team,
                    "org" => Visibility::Org,
                    "public" => Visibility::Public,
                    _ => Visibility::Private,
                };
                assert!(
                    admissible_independently(label, scope),
                    "a {label:?} node was exported at {scope:?} scope"
                );
            }
        }
    }

    /// An edge whose endpoint was filtered must go too — otherwise the edge
    /// discloses that a hidden entity exists.
    #[test]
    fn an_edge_to_a_filtered_node_is_dropped_even_if_the_edge_is_admissible() {
        // The public edge 4->1 points at Ada, who is private.
        let g = export_neighborhood(&mixed(), Visibility::Public);
        let ids: std::collections::HashSet<&str> = g.nodes.iter().map(|n| n.id.as_str()).collect();
        for e in &g.edges {
            assert!(
                ids.contains(e.from.as_str()) && ids.contains(e.to.as_str()),
                "edge {e:?} references a node absent from the export"
            );
        }
        assert!(
            g.edges.is_empty(),
            "the only public edge points at a private entity, so nothing should \
             survive: {:?}",
            g.edges
        );
    }

    #[test]
    fn a_private_scope_keeps_everything_and_filters_nothing() {
        let g = export_neighborhood(&mixed(), Visibility::Private);
        assert_eq!(g.meta.node_count, 6, "4 entities + 2 documents");
        assert_eq!(g.meta.edge_count, 4);
        assert_eq!(g.meta.filtered_out, 0);
    }

    /// Descriptions are free text and are deliberately not part of the format.
    #[test]
    fn entity_descriptions_are_never_exported() {
        let json = export_neighborhood(&mixed(), Visibility::Private)
            .to_json_pretty()
            .unwrap();
        assert!(
            !json.contains("must never be exported"),
            "an entity description leaked into the export"
        );
    }

    /// Byte-stability, so regenerated imagery matches its published counts.
    #[test]
    fn the_same_neighborhood_exports_byte_identically() {
        let a = export_neighborhood(&mixed(), Visibility::Private)
            .to_json_pretty()
            .unwrap();
        let b = export_neighborhood(&mixed(), Visibility::Private)
            .to_json_pretty()
            .unwrap();
        assert_eq!(a, b);
    }

    /// Ordering must not depend on the traversal's discovery order, or two
    /// renders of one graph disagree.
    #[test]
    fn export_order_is_independent_of_input_order() {
        let forward = export_neighborhood(&mixed(), Visibility::Private)
            .to_json_pretty()
            .unwrap();

        let mut shuffled = mixed();
        shuffled.entities.reverse();
        shuffled.triples.reverse();
        shuffled.documents.reverse();
        let reversed = export_neighborhood(&shuffled, Visibility::Private)
            .to_json_pretty()
            .unwrap();

        assert_eq!(forward, reversed, "export order follows input order");
    }

    /// The truncation flag must survive into the file — a renderer needs to be
    /// able to say "partial view" rather than implying completeness.
    #[test]
    fn the_truncation_flag_is_carried_into_the_export() {
        let mut n = mixed();
        n.truncated = true;
        assert!(export_neighborhood(&n, Visibility::Private).meta.truncated);

        let json = export_neighborhood(&n, Visibility::Public)
            .to_json_pretty()
            .unwrap();
        assert!(json.contains("\"truncated\": true"), "got {json}");
    }

    #[test]
    fn counts_in_meta_match_the_arrays_they_describe() {
        for scope in [Visibility::Private, Visibility::Org, Visibility::Public] {
            let g = export_neighborhood(&mixed(), scope);
            assert_eq!(g.meta.node_count, g.nodes.len());
            assert_eq!(g.meta.edge_count, g.edges.len());
            assert_eq!(g.meta.entity_count + g.meta.document_count, g.nodes.len());
        }
    }

    /// A renderer colours or groups by `entity_type`, so it must be present on
    /// entities and absent on documents. Found by mutation: setting it to `None`
    /// unconditionally passed every other test here.
    #[test]
    fn entity_type_is_present_on_entities_and_absent_on_documents() {
        let g = export_neighborhood(&mixed(), Visibility::Private);
        let entities: Vec<_> = g
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Entity)
            .collect();
        let documents: Vec<_> = g
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Document)
            .collect();
        assert_eq!(entities.len(), 4);
        assert_eq!(documents.len(), 2);
        for n in entities {
            assert_eq!(
                n.entity_type.as_deref(),
                Some("person"),
                "entity {:?} lost its type",
                n.label
            );
        }
        for n in documents {
            assert!(n.entity_type.is_none(), "a document carried an entity_type");
            assert!(n.weight.is_none(), "a document carried a weight");
        }
    }

    #[test]
    fn ids_are_lowercase_hex_of_the_expected_width() {
        let g = export_neighborhood(&mixed(), Visibility::Private);
        for n in &g.nodes {
            assert_eq!(n.id.len(), 64, "{:?}", n.id);
            assert!(n
                .id
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()));
        }
    }
}
