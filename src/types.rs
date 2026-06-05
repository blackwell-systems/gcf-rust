use serde::{Deserialize, Serialize};

/// Score breakdown for a symbol.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Components {
    #[serde(default)]
    pub blast_radius: f64,
    #[serde(default)]
    pub confidence: f64,
    #[serde(default)]
    pub recency: f64,
    #[serde(default)]
    pub distance: f64,
}

/// A node in a GCF payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Symbol {
    pub qualified_name: String,
    pub kind: String,
    pub score: f64,
    pub provenance: String,
    #[serde(default)]
    pub distance: i32,
    #[serde(default)]
    pub signature: String,
    #[serde(default)]
    pub components: Components,
}

/// A directed relationship in a GCF payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub source: String,
    pub target: String,
    pub edge_type: String,
    #[serde(default)]
    pub status: String,
}

/// The input/output structure for GCF encoding/decoding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Payload {
    pub tool: String,
    #[serde(default)]
    pub tokens_used: i64,
    #[serde(default)]
    pub token_budget: i64,
    #[serde(default)]
    pub pack_root: String,
    #[serde(default)]
    pub symbols: Vec<Symbol>,
    #[serde(default)]
    pub edges: Vec<Edge>,
}

/// Represents the diff between a prior context pack and the current result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeltaPayload {
    pub tool: String,
    pub base_root: String,
    pub new_root: String,
    #[serde(default)]
    pub removed: Vec<Symbol>,
    #[serde(default)]
    pub added: Vec<Symbol>,
    #[serde(default)]
    pub removed_edges: Vec<Edge>,
    #[serde(default)]
    pub added_edges: Vec<Edge>,
    #[serde(default)]
    pub delta_tokens: i64,
    #[serde(default)]
    pub full_tokens: i64,
}
