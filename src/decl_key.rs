//! Identity of a dolfin declaration, shared between turtle generation
//! (`irukame`, dolfin -> ttl output ranges) and turtle import (`mekarui`,
//! ttl -> dolfin provenance) so both sides key the same declaration alike.

use serde::{Deserialize, Serialize};

/// What kind of declaration a [`DeclKey`] names.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum DeclKind {
    Concept,
    Property,
    /// A `has` field of concept `owner`.
    Field { owner: String },
    Fact,
    /// A `one_of` variant of concept `owner`.
    EnumVariant { owner: String },
    /// A top-level `rule`, live N3 or commented out (nested rules included).
    Rule,
}

/// A declaration: `file` is the ontology's package-relative path (as in
/// `OntologyFile::relative_path`), `name` its local name (variant/field name
/// for `Field`/`EnumVariant`, fact id for `Fact`, rule name for `Rule`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DeclKey {
    pub file: String,
    pub kind: DeclKind,
    pub name: String,
}
