//! Code generation for Dolfin.
//!
//! This crate provides code generators that transform a loaded Dolfin package
//! into various output formats.

pub mod decl_key;
pub mod plugin;
mod turtle;

pub use decl_key::{DeclKey, DeclKind};
pub use plugin::{DolfinPlugin, PluginError};
pub use turtle::{namespace_iri, namespace_prefix_iri, TurtleError, TurtleGenerator, TurtleOptions};

/// Generate N3 rules from a dolfin package.
///
/// Returns a string of valid N3, ready for a rule engine.
/// Equivalent to `TurtleGenerator::with_defaults().generate_n3_rules(package)`.
pub fn rules_as_n3(package: &rowl::package::Package) -> Result<String, TurtleError> {
    TurtleGenerator::with_defaults().generate_n3_rules(package)
}
