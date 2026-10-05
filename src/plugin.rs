//! Plugin trait for Dolfin code-generation plugins.
//!
//! A plugin reads `#@` annotations from a loaded [`Package`] (which now
//! carries a [`CommentMap`] per ontology file) and transforms annotated AST
//! sub-trees into additional output artefacts (SHACL, SparNatural config, …).
//!
//! Parsers and tools that do not have a plugin installed simply ignore any
//! `#@` comments — the core Dolfin language is unaffected.

use rowl::package::Package;
use thiserror::Error;

/// Errors that can occur while a plugin is running.
#[derive(Debug, Error)]
pub enum PluginError {
    #[error("Formatting error: {0}")]
    FormatError(#[from] std::fmt::Error),

    #[error("Plugin error: {0}")]
    Other(String),
}

/// A Dolfin code-generation plugin.
///
/// Implement this trait to add a new output target driven by `#@` annotations.
/// The plugin receives the fully-loaded [`Package`] (AST + attached
/// [`CommentMap`] per file) and returns the generated text output.
pub trait DolfinPlugin {
    /// The annotation name this plugin claims (e.g. `"sparnatural"`).
    ///
    /// Only `#@` comments whose first token matches this name will be
    /// relevant to the plugin; all others are ignored.
    fn annotation_name(&self) -> &str;

    /// Run the plugin over the package and return the generated output.
    fn run(&self, package: &Package) -> Result<String, PluginError>;
}
