//! Code generation for Dolfin.
//!
//! This crate provides code generators that transform a loaded Dolfin package
//! into various output formats.

pub mod plugin;
mod turtle;

pub use plugin::{DolfinPlugin, PluginError};
pub use turtle::{namespace_iri, TurtleError, TurtleGenerator, TurtleOptions};

/// Generate N3 rules from a dolfin package.
///
/// Returns a string of valid N3 that can be fed to `retox::load_n3_rules_from_str`.
/// Equivalent to `TurtleGenerator::with_defaults().generate_n3_rules(package)`.
pub fn rules_as_n3(package: &rowl::package::Package) -> Result<String, TurtleError> {
    TurtleGenerator::with_defaults().generate_n3_rules(package)
}
