// dolfin-codegen/src/turtle.rs
//! Turtle (RDF/OWL) code generation for Dolfin packages.

use rowl::annotation::parse_annotation;
use dolfin_query::to_sparql;
use rowl::ast::{
    Cardinality, ConceptDef, Declaration, FactAssertion, FactDef, FactValue, HasDeclaration,
    IriNameValue, PrimitiveKind, PropertyAxiom, PropertyDef, PropertyPath, QualifiedName, RuleDef,
    TypeRef,
};
use rowl::comment::Comment as RawComment;
use rowl::error::{Location, Span as RawSpan};
use rowl::package::{OntologyFile, Package};
use rowl::{
    BinaryOp, ComparisonOp, Constraint, ConstraintBlock, Expr, Literal, Object, Pattern, Subject,
    ThenItem, UnaryOp,
};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write;
use thiserror::Error;

/// Base IRI for dolfin quantity terms (`dq:coefficient`, `dq:dimension`, …).
const DQ_IRI_BASE: &str = "https://dolfin.dev/quantity#";

/// Flatten an associative same-op binary chain into its leaf operands.
///
/// For an associative operator (`+`, `*`), `((a op b) op c)` is collected as
/// `[a, b, c]`. Descent stops at any node that is not a `BinaryOp` with the
/// same `op`, so sub-expressions of a different operator are left intact for
/// the caller to render recursively.
fn collect_chain<'a>(op: &BinaryOp, expr: &'a Expr, out: &mut Vec<&'a Expr>) {
    if let Expr::BinaryOp {
        op: child_op,
        left,
        right,
        ..
    } = expr
    {
        if child_op == op {
            collect_chain(op, left, out);
            collect_chain(op, right, out);
            return;
        }
    }
    out.push(expr);
}

/// Errors that can occur during Turtle generation.
#[derive(Debug, Error)]
pub enum TurtleError {
    #[error("Unresolved type reference: {0}")]
    UnresolvedType(String),

    #[error("Unresolved property reference: {0}")]
    UnresolvedProperty(String),

    #[error("Formatting error: {0}")]
    FormatError(#[from] std::fmt::Error),
}

/// Options for Turtle generation.
#[derive(Debug, Clone)]
pub struct TurtleOptions {
    /// Base IRI for the generated ontology.
    pub base_iri: String,

    /// Include comments in output.
    pub include_comments: bool,

    /// Include rules as comments (since SWRL may not be universally supported).
    pub include_rules_as_comments: bool,

    /// Include queries transpiled to SPARQL as comments.
    pub include_queries_as_comments: bool,
}

impl Default for TurtleOptions {
    fn default() -> Self {
        Self {
            base_iri: "http://example.org/".to_string(),
            include_comments: true,
            include_rules_as_comments: true,
            include_queries_as_comments: true,
        }
    }
}

// Pieces of informations needed to resolve name locally
struct GeneratorContext<'a> {
    namespace: &'a QualifiedName,
    ontology: &'a OntologyFile,
    package: &'a Package,
}
/// A flattened rule ready for N3 emission.
struct FlatN3Rule {
    name: String,
    match_patterns: Vec<Pattern>,
    then_items: Vec<ThenItem>, // only non-nested items
}

/// SKOS metadata extracted for a single entity.
///
/// `comments` carries the leading doc-comment groups (one per language) that
/// become `rdfs:comment` triples; `definitions` carries the `#@ glossary:`
/// `definition=` / `definition@<lang>=` arguments that become `skos:definition`
/// triples; `labels`/`alt_labels` carry `label`/`prefLabel`/`pref_label` and
/// `alt_label`/`altLabel` (each `[@<lang>]=`) becoming `skos:prefLabel` /
/// `skos:altLabel` triples. All four are `(Option<lang>, text)` — `None` means
/// an untagged literal. `labels` is never empty: it falls back to `[(None,
/// name)]` when no `#@ glossary:` block overrides it.
struct SkosData {
    labels: Vec<(Option<String>, String)>,
    comments: Vec<(Option<String>, String)>,
    definitions: Vec<(Option<String>, String)>,
    alt_labels: Vec<(Option<String>, String)>,
    scope_note: String,
}

/// If `type_ref` is a `Named` reference whose last name segment matches a
/// well-known physical dimension (`unit.Mass`, `Speed`, …, see
/// `dolfin_units::named_dimension`), the IRI to use as `rdfs:range` instead
/// of the referenced concept. The property holds a `quantity(...)` literal,
/// not an instance of that concept, so its range names the dimension rather
/// than the concept's class IRI.
fn dimension_range_iri(type_ref: &TypeRef) -> Option<String> {
    let TypeRef::Named { name, .. } = type_ref else {
        return None;
    };
    let dim = dolfin_units::named_dimension(&name.last())?;
    Some(format!("{}{}", dolfin_units::DIMENSION_IRI_BASE, dim.canonical_string()))
}

/// The `rdfs:range` IRI for a dimension-typed `has`/`property` reference,
/// preferring a real-world IRI over the minted `dolfin.dev/dimension/...`
/// fallback. Precedence, highest first:
///
/// 1. An explicit `@iri_name <...>` override on a concept in the package
///    named after the dimension (e.g. a local `concept Mass: @iri_name <...>`)
///    — lets a project pin its own IRI. Searches every ontology file in the
///    package, since such a concept usually lives in a separate file
///    (`units.dlf`) from the property referencing it.
/// 2. [`dolfin_units::qudt_quantity_kind_iri`] — the built-in QUDT
///    quantity-kind IRI, known to the tooling directly; no `units.dlf` (or
///    any concept declaration) needs to exist in the project for this.
/// 3. [`dimension_range_iri`]'s minted `dolfin.dev` IRI, for the rare case a
///    well-known dimension has no QUDT mapping.
fn dimension_range_iri_for_ctx(type_ref: &TypeRef, ctx: &GeneratorContext) -> Option<String> {
    let TypeRef::Named { name, .. } = type_ref else {
        return None;
    };
    let dim_name = name.last();
    dolfin_units::named_dimension(&dim_name)?;

    for (_, ontology) in ctx.package.iter_ontologies() {
        for decl in &ontology.ast.declarations {
            if let Declaration::Concept(c) = decl {
                if *c.name.get() == dim_name {
                    if let Some(IriNameValue::AbsoluteUri(iri)) = &c.iri_name {
                        return Some(iri.clone());
                    }
                }
            }
        }
    }

    dolfin_units::qudt_quantity_kind_iri(&dim_name).or_else(|| dimension_range_iri(type_ref))
}

/// Turtle code generator.
pub struct TurtleGenerator {
    options: TurtleOptions,
    blank_counter: u64,
    var_counter: u64,
    /// When true, `Declaration::Fact` declarations are skipped during emission.
    /// Used by `generate_schema_only` to emit the T-box without ABox fact
    /// individuals, so the two can be counted separately.
    skip_facts: bool,
    /// Temporal resolution context for the ontology file currently being
    /// written, built from its `@locale` / `@timezone` directives. Reset per
    /// file in `write_ontology_file`; strict (no defaults) otherwise.
    temporal_ctx: dolfin_datetime::TemporalContext,
    /// Physical-quantity units encountered while rendering the current output,
    /// keyed by canonical unit string → (coefficient-to-SI, dimension string,
    /// display symbol). Populated by `render_literal` (hence `RefCell`, since
    /// it runs behind `&self`); drives the conditional `unit:`/`dq:` prefixes
    /// and the once-per-unit definition block. Cleared at the start of each
    /// `generate` / `generate_n3_rules`.
    unit_defs: RefCell<BTreeMap<String, (f64, String, String)>>,
    /// Project-scoped unit registry, built once per `generate`/`generate_n3_rules`
    /// call from the package's `unit` declarations (see `dolfin_analysis::units`)
    /// — lets `render_literal` resolve `quantity(...)` literals against
    /// project-declared currencies and nominal/family units, not just builtins.
    unit_registry: RefCell<dolfin_units::UnitRegistry>,
}

impl TurtleGenerator {
    /// Create a new generator with the given options.
    pub fn new(options: TurtleOptions) -> Self {
        Self {
            options,
            blank_counter: 0,
            var_counter: 0,
            skip_facts: false,
            temporal_ctx: dolfin_datetime::TemporalContext::strict(),
            unit_defs: RefCell::new(BTreeMap::new()),
            unit_registry: RefCell::new(dolfin_units::UnitRegistry::with_defaults()),
        }
    }

    /// Create a new generator with default options.
    pub fn with_defaults() -> Self {
        Self::new(TurtleOptions::default())
    }

    /// Generate Turtle output for a package.
    pub fn generate(&mut self, package: &Package) -> Result<String, TurtleError> {
        self.unit_defs.borrow_mut().clear();
        self.rebuild_unit_registry(package);
        let mut output = String::new();

        // Write header
        self.write_header(&mut output, package)?;

        // Write prefixes
        let seen_prefixes = self.write_prefixes(&mut output, package)?;

        // The declarations are rendered into a separate buffer first so that
        // `render_literal` can populate `unit_defs`; the `unit:`/`dq:` prefixes
        // are then declared only when a quantity was actually emitted, keeping
        // quantity-free output byte-identical.
        let mut body = String::new();
        self.write_ontology_declaration(&mut body, package)?;
        for (namespace, ontology) in package.iter_ontologies() {
            let mut ontology = ontology.clone();
            for (prefix, _) in &seen_prefixes {
                ontology.resolved_prefixes.entry(prefix.to_string()).or_insert_with(|| {
                    package.namespace().join(&QualifiedName {
                        parts: vec![prefix.clone()],
                        is_prefixed: false,
                        span: None,
                    })
                });
            }
            let ctx = GeneratorContext {
                namespace,
                ontology: &ontology,
                package,
            };
            self.write_ontology_file(&mut body, &ctx)?;
        }

        if !self.unit_defs.borrow().is_empty() {
            self.write_quantity_prefixes(&mut output)?;
        }
        output.push_str(&body);
        self.write_unit_definitions(&mut output)?;

        Ok(output)
    }

    /// Rebuild `unit_registry` from every `unit` declaration across the
    /// package's ontology files (see `dolfin_analysis::units::build_registry`).
    /// Resolution errors (e.g. a derived unit referencing an unknown unit)
    /// are silently dropped here — they're reported as compile diagnostics by
    /// the analysis pass, not by Turtle generation.
    fn rebuild_unit_registry(&self, package: &Package) {
        let files: Vec<_> = package.iter_ontologies().map(|(_, ontology)| &ontology.ast).collect();
        let (registry, _errors) = dolfin_analysis::units::build_registry(files);
        *self.unit_registry.borrow_mut() = registry;
    }

    /// Generate Turtle for a package **excluding** `fact` declarations.
    ///
    /// Identical to [`generate`](Self::generate) but the ABox individuals
    /// produced by `fact` blocks are omitted. Counting the triples of this
    /// output and subtracting from the full output yields the number of
    /// fact-derived (data) triples, letting callers report schema (T-box) and
    /// data (fact ABox) triple counts separately.
    pub fn generate_schema_only(&mut self, package: &Package) -> Result<String, TurtleError> {
        let prev = self.skip_facts;
        self.skip_facts = true;
        let result = self.generate(package);
        self.skip_facts = prev;
        result
    }

    /// Generate N3 rules output for a package.
    ///
    /// Emits only the `@prefix` declarations and rule bodies as valid N3, with
    /// no ontology header, concept/property triples, or comment prefixes.
    /// The output can be fed directly to `retox::load_n3_rules_from_str`.
    ///
    /// `include_rules_as_comments` is ignored — rules are always emitted as
    /// real N3, not comments.
    pub fn generate_n3_rules(&mut self, package: &Package) -> Result<String, TurtleError> {
        self.unit_defs.borrow_mut().clear();
        self.rebuild_unit_registry(package);
        let prev = self.options.include_rules_as_comments;
        self.options.include_rules_as_comments = false;

        let mut out = String::new();
        let seen_prefixes = self.write_prefixes(&mut out, package)?;

        // Rule bodies into a buffer first (see `generate`), so the `unit:`/`dq:`
        // prefixes are declared only when a rule actually compares a quantity.
        let mut body = String::new();
        for (namespace, ontology) in package.iter_ontologies() {
            let mut ontology = ontology.clone();
            for (prefix, _) in &seen_prefixes {
                ontology.resolved_prefixes.entry(prefix.to_string()).or_insert_with(|| {
                    package.namespace().join(&QualifiedName {
                        parts: vec![prefix.clone()],
                        is_prefixed: false,
                        span: None,
                    })
                });
            }
            let ctx = GeneratorContext { namespace, ontology: &ontology, package };
            for decl in &ctx.ontology.ast.declarations {
                if let Declaration::Rule(rule) = decl {
                    self.write_rule(&mut body, rule, &ctx)?;
                }
            }
        }

        if !self.unit_defs.borrow().is_empty() {
            self.write_quantity_prefixes(&mut out)?;
        }
        out.push_str(&body);
        // retox resolves a `unit:` datatype via its own registry, so the N3
        // rules output needs no `dq:` definition block.

        self.options.include_rules_as_comments = prev;
        Ok(out)
    }

    // ── SKOS helpers ─────────────────────────────────────────────────────────

    /// Extract SKOS metadata for an entity identified by `name` and `span`.
    ///
    /// Two independent carriers, no cross-fallback (see clarification.md
    /// "Doc comments, languages, and the `#xx>` prefix"):
    /// - Leading non-`@` comment lines → language-grouped `rdfs:comment` groups
    ///   (`#xx> …` tags a language; a plain `# …` line is untagged).
    /// - `#@ glossary:` `definition=` / `definition@xx=` args → `skos:definition`
    ///   literals; `label=` / `alt_label=` / `scope_note=` → the SKOS labels.
    /// - The entity `name` → `skos:prefLabel`.
    /// A leading comment never becomes `skos:definition`, and a `definition=`
    /// arg never becomes `rdfs:comment`.
    ///
    /// Note: if a description comment and `#@ glossary` appear on adjacent lines
    /// they are merged by the lexer into a single comment token. This method
    /// handles that case by splitting the merged text line-by-line.
    fn extract_skos_data(
        &self,
        name: &str,
        span: Option<RawSpan>,
        ontology: &OntologyFile,
    ) -> SkosData {
        let Some(span) = span else {
            return SkosData {
                labels: vec![(None, name.to_string())],
                comments: vec![],
                definitions: vec![],
                alt_labels: vec![],
                scope_note: String::new(),
            };
        };

        let leading = ontology.comment_map.leading_comments(&span);

        // Split all leading comment lines into language-grouped description
        // groups (leading doc comment -> rdfs:comment) and an optional
        // annotation block. Handles both merged and separate comments.
        let (comment_groups, gloss_ann) = parse_leading_for_glossary(leading);

        // Also try the fallback line-before scan (same as SparNatural) when the
        // comment map missed the annotation due to edge-case classification.
        // This covers the case where a blank line separates the annotation block
        // from a description comment, pushing the annotation into dangling.
        let gloss_ann = gloss_ann.or_else(|| {
            let target_line = span.start.line;
            let first_leading_line = leading.iter().map(|c| c.line).min();
            let all = ontology
                .comment_map
                .leading
                .values()
                .flatten()
                .chain(ontology.comment_map.inside.values().flatten())
                .chain(ontology.comment_map.dangling.iter());
            for c in all {
                if c.line >= target_line {
                    continue;
                }
                // Count only the '@'-prefixed lines: when annotation and description
                // are adjacent the lexer merges them into one token, so using the
                // total line count would extend ann_end_line past the annotation block.
                let ann_line_count = c
                    .text
                    .lines()
                    .take_while(|l| l.trim().starts_with('@'))
                    .count();
                if ann_line_count == 0 {
                    continue;
                }
                let ann_end_line = c.line + ann_line_count - 1;
                let just_before_concept = ann_end_line + 1 == target_line;
                let just_before_leading = first_leading_line
                    .map_or(false, |fl| ann_end_line + 1 == fl || ann_end_line + 2 == fl);
                if (just_before_concept || just_before_leading)
                    && let Some(ann) = parse_annotation(c)
                    && ann.name == "glossary"
                {
                    return Some(ann);
                }
            }
            None
        });

        if let Some(ann) = gloss_ann {
            // Every `label=`/`prefLabel=`/`pref_label=` (untagged) or
            // `label@<lang>=`/`prefLabel@<lang>=`/`pref_label@<lang>=` (tagged)
            // argument becomes one skos:prefLabel literal, in source order.
            let mut labels: Vec<(Option<String>, String)> = ann
                .args
                .iter()
                .filter_map(|(k, v)| {
                    if k == "label" || k == "prefLabel" || k == "pref_label" {
                        Some((None, v.clone()))
                    } else if let Some(lang) = k
                        .strip_prefix("label@")
                        .or_else(|| k.strip_prefix("prefLabel@"))
                        .or_else(|| k.strip_prefix("pref_label@"))
                    {
                        is_lang_tag(lang).then(|| (Some(lang.to_string()), v.clone()))
                    } else {
                        None
                    }
                })
                .collect();
            if labels.is_empty() {
                labels.push((None, name.to_string()));
            }
            // Every `definition=` (untagged) or `definition@<lang>=` (tagged)
            // argument becomes one skos:definition literal, in source order.
            let definitions = ann
                .args
                .iter()
                .filter_map(|(k, v)| {
                    if k == "definition" {
                        Some((None, v.clone()))
                    } else if let Some(lang) = k.strip_prefix("definition@") {
                        is_lang_tag(lang).then(|| (Some(lang.to_string()), v.clone()))
                    } else {
                        None
                    }
                })
                .collect();
            let alt_labels = ann
                .args
                .iter()
                .filter_map(|(k, v)| {
                    if k == "alt_label" || k == "altLabel" {
                        Some((None, v.clone()))
                    } else if let Some(lang) =
                        k.strip_prefix("alt_label@").or_else(|| k.strip_prefix("altLabel@"))
                    {
                        is_lang_tag(lang).then(|| (Some(lang.to_string()), v.clone()))
                    } else {
                        None
                    }
                })
                .collect();
            let scope_note = ann.arg_with_alt(&["scopeNote", "scope_note"]).unwrap_or("").to_string();
            SkosData {
                labels,
                comments: comment_groups,
                definitions,
                alt_labels,
                scope_note,
            }
        } else {
            SkosData {
                labels: vec![(None, name.to_string())],
                comments: comment_groups,
                definitions: vec![],
                alt_labels: vec![],
                scope_note: String::new(),
            }
        }
    }

    /// Append `skos:` predicate triples for `data` to the current subject block.
    /// Each line is a `;`-continuation so the caller must write the closing `.`.
    fn write_skos_triples(&self, out: &mut String, data: &SkosData) -> Result<(), TurtleError> {
        for (lang, text) in &data.labels {
            match lang {
                Some(l) => writeln!(
                    out,
                    "  ; skos:prefLabel \"{}\"@{}",
                    escape_turtle_string(text),
                    l
                )?,
                None => writeln!(out, "  ; skos:prefLabel \"{}\"", escape_turtle_string(text))?,
            }
        }
        for (lang, text) in &data.comments {
            let text = glossary_ir::links::braces_to_backticks(text);
            match lang {
                Some(l) => writeln!(
                    out,
                    "  ; rdfs:comment \"{}\"@{}",
                    escape_turtle_string(&text),
                    l
                )?,
                None => writeln!(out, "  ; rdfs:comment \"{}\"", escape_turtle_string(&text))?,
            }
        }
        for (lang, text) in &data.definitions {
            let text = glossary_ir::links::braces_to_backticks(text);
            match lang {
                Some(l) => writeln!(
                    out,
                    "  ; skos:definition \"{}\"@{}",
                    escape_turtle_string(&text),
                    l
                )?,
                None => writeln!(
                    out,
                    "  ; skos:definition \"{}\"",
                    escape_turtle_string(&text)
                )?,
            }
        }
        for (lang, text) in &data.alt_labels {
            match lang {
                Some(l) => writeln!(
                    out,
                    "  ; skos:altLabel \"{}\"@{}",
                    escape_turtle_string(text),
                    l
                )?,
                None => writeln!(out, "  ; skos:altLabel \"{}\"", escape_turtle_string(text))?,
            }
        }
        if !data.scope_note.is_empty() {
            writeln!(
                out,
                "  ; skos:scopeNote \"{}\"",
                escape_turtle_string(&glossary_ir::links::braces_to_backticks(&data.scope_note))
            )?;
        }
        Ok(())
    }

    /// Write the Turtle header comment.
    fn write_header(&self, out: &mut String, package: &Package) -> Result<(), TurtleError> {
        if self.options.include_comments {
            writeln!(out, "# Generated by Irukame")?;
            writeln!(out, "# IRUKA   = イルカ = dolphin 🐬")?;
            writeln!(out, "#    KAME = 　カメ = turtle  🐢)")?;
            writeln!(out, "# Package: {}", self.namespace_to_iri(package.namespace()))?;
            writeln!(out, "# Version: {}", package.version())?;
            writeln!(out)?;
        }
        Ok(())
    }

    /// Write all prefix declarations.
    fn write_prefixes(
        &self,
        out: &mut String,
        package: &Package,
    ) -> Result<HashMap<String, String>, TurtleError> {
        // Standard prefixes
        writeln!(
            out,
            "@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> ."
        )?;
        writeln!(
            out,
            "@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> ."
        )?;
        writeln!(out, "@prefix owl: <http://www.w3.org/2002/07/owl#> .")?;
        writeln!(out, "@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .")?;
        writeln!(
            out,
            "@prefix skos: <http://www.w3.org/2004/02/skos/core#> ."
        )?;
        writeln!(
            out,
            "@prefix dc: <http://purl.org/dc/elements/1.1/> ."
        )?;
        writeln!(out)?;

        // Base namespace (empty prefix)
        let base_iri = self.namespace_to_iri(package.namespace());
        writeln!(out, "@prefix : <{}#> .", base_iri)?;

        // Collect all unique namespace prefixes
        let mut seen_prefixes: HashMap<String, String> = HashMap::new();
        seen_prefixes.insert(String::new(), format!("{}#", base_iri)); // Empty prefix already declared

        for (namespace, ontology) in package.iter_ontologies() {
            // Prefix for this ontology's namespace
            let prefix = self.namespace_to_prefix(namespace, package.namespace());
            if !prefix.is_empty() && !seen_prefixes.contains_key(&prefix) {
                let iri =
                    self.namespace_to_iri_with_override(namespace, ontology.iri_name.as_ref());
                writeln!(out, "@prefix {}: <{}#> .", prefix, iri)?;
                seen_prefixes.insert(prefix, format!("{}#", iri));
            }

            // Prefixes from imported namespaces
            for (alias, resolved_ns) in &ontology.resolved_prefixes {
                if !seen_prefixes.contains_key(alias) {
                    if resolved_ns.parts.len() == 1 && resolved_ns.parts[0].contains("://") {
                        // URI-literal prefix: emit verbatim, no base_iri prepend or # suffix
                        let uri = &resolved_ns.parts[0];
                        writeln!(out, "@prefix {}: <{}> .", alias, uri)?;
                        seen_prefixes.insert(alias.to_string(), uri.clone());
                    } else {
                        let iri = self.namespace_to_iri(resolved_ns);
                        writeln!(out, "@prefix {}: <{}#> .", alias, iri)?;
                        seen_prefixes.insert(alias.to_string(), format!("{}#", iri));
                    }
                }
            }
        }
        writeln!(
            out,
            "@prefix math: <http://www.w3.org/2000/10/swap/math#> ."
        )?;
        writeln!(out)?;
        Ok(seen_prefixes)
    }

    /// Write the main ontology declaration.
    fn write_ontology_declaration(
        &self,
        out: &mut String,
        package: &Package,
    ) -> Result<(), TurtleError> {
        let base_iri = self.namespace_to_iri(package.namespace());

        writeln!(out, "<{}> rdf:type owl:Ontology", base_iri)?;

        if let Some(description) = &package.manifest.description {
            writeln!(
                out,
                "  ; rdfs:comment \"{}\"",
                escape_turtle_string(&glossary_ir::links::braces_to_backticks(description))
            )?;
        }

        if let Some(author) = &package.manifest.author {
            writeln!(
                out,
                "  ; dc:creator \"{}\"",
                escape_turtle_string(author)
            )?;
        }

        writeln!(out, "  ; owl:versionInfo \"{}\" .", package.version())?;
        writeln!(out)?;

        Ok(())
    }

    /// Write all declarations from an ontology file.
    fn write_ontology_file(
        &mut self,
        out: &mut String,
        ctx: &GeneratorContext,
    ) -> Result<(), TurtleError> {
        // Build the temporal context for this file from its @locale/@timezone
        // directives. On a malformed directive, fall back to strict; the
        // dolfin-analysis validation pass reports the directive error.
        self.temporal_ctx = ctx.ontology.ast.temporal_context().unwrap_or_default();

        if self.options.include_comments {
            writeln!(out, "# ============================================")?;
            writeln!(out, "# Ontology: {}", self.namespace_to_iri(ctx.namespace))?;
            if let Some(iri_name) = &ctx.ontology.iri_name {
                writeln!(out, "# IRI Name: {}", iri_name.as_str())?;
            }
            writeln!(out, "# File: {}", ctx.ontology.relative_path.display())?;
            writeln!(out, "# ============================================")?;
            writeln!(out)?;
        }

        // Declare the file resource and link it to the package ontology. This
        // enumerates the package's file set for the reverse direction; base-
        // namespace entities are defined by the package IRI itself, so no
        // self-loop is emitted for the base file. When the file has an
        // `@iri_name` override, `file_resource` is the override IRI (used by
        // every entity's `isDefinedBy`) and a second hop links it back to the
        // canonical, path-derived IRI so mekarui can still recover the file's
        // on-disk path.
        let file_resource = self.file_resource_iri(ctx);
        let canonical = self.namespace_to_iri(ctx.namespace);
        let package_iri = self.namespace_to_iri(ctx.package.namespace());
        let mut wrote_link = false;
        if file_resource != canonical {
            writeln!(out, "<{}> rdfs:isDefinedBy <{}> .", file_resource, canonical)?;
            wrote_link = true;
        }
        if canonical != package_iri {
            writeln!(out, "<{}> rdfs:isDefinedBy <{}> .", canonical, package_iri)?;
            wrote_link = true;
        }
        if wrote_link {
            writeln!(out)?;
        }

        for decl in &ctx.ontology.ast.declarations {
            match decl {
                Declaration::Concept(concept) => {
                    self.write_concept(out, concept, ctx)?;
                }
                Declaration::Property(property) => {
                    self.write_property(out, property, ctx)?;
                }
                Declaration::Rule(rule) => {
                    self.write_rule(out, rule, ctx)?;
                }
                Declaration::Fact(fact) => {
                    if !self.skip_facts {
                        self.write_fact(out, fact, ctx)?;
                    }
                }
                Declaration::Query(query) => {
                    if self.options.include_queries_as_comments {
                        self.write_query(out, query, ctx)?;
                    }
                }
                // Unit declarations feed the project-wide unit registry (see
                // dolfin-analysis::units), not Turtle T-box/A-box output.
                Declaration::Unit(_) => {}
            }
        }

        Ok(())
    }

    /// Write a concept definition.
    fn write_concept(
        &self,
        out: &mut String,
        concept: &ConceptDef,
        ctx: &GeneratorContext,
    ) -> Result<(), TurtleError> {
        let prefix = self.namespace_to_prefix(ctx.namespace, ctx.package.namespace());
        let any_concept_iri_name = ctx.ontology.ast.declarations.iter().any(|d| {
            matches!(d, Declaration::Concept(c) if c.iri_name.is_some())
        });
        let class_ref = if let Some(IriNameValue::AbsoluteUri(iri)) = &concept.iri_name {
            format!("<{}>", iri)
        } else if any_concept_iri_name {
            let ns_iri = self.namespace_to_iri(ctx.namespace);
            format!("<{}#{}>", ns_iri, concept.name.get())
        } else if prefix.is_empty() {
            format!(":{}", concept.name.get())
        } else {
            format!("{}:{}", prefix, concept.name.get())
        };

        if self.options.include_comments {
            writeln!(out, "# Concept: {}", concept.name.get())?;
        }

        writeln!(out, "{} rdf:type rdfs:Class", class_ref)?;

        // Handle sub declarations (inheritance)
        let mut has_sub = false;
        for parent_type in &concept.parents {
            let parent_ref = self.type_ref_to_turtle(parent_type, ctx)?;
            if has_sub {
                writeln!(out, "    , {}", parent_ref)?;
            } else {
                writeln!(out, "  ; rdfs:subClassOf {}", parent_ref)?;
                has_sub = true;
            }
        }

        let skos = self.extract_skos_data(concept.name.get(), concept.span, ctx.ontology);
        self.write_skos_triples(out, &skos)?;

        writeln!(out, "  ; rdfs:isDefinedBy <{}>", self.file_resource_iri(ctx))?;
        writeln!(out, "  .")?;
        writeln!(out)?;

        // Handle has declarations (properties)
        for has in &concept.has_declarations {
            self.write_has_property(out, has, &class_ref, ctx)?;
        }

        // Handle one_of variants as named individuals
        if let Some(variants) = &concept.one_of {
            if !variants.is_empty() {
                // Restrict class to enumeration of named individuals
                write!(
                    out,
                    "{} owl:equivalentClass [ rdf:type owl:Class ; owl:oneOf (",
                    class_ref
                )?;
                for variant in variants {
                    write!(out, " {}:{}_{}", prefix, concept.name.get(), variant.name)?;
                }
                writeln!(out, " ) ] .")?;
                writeln!(out)?;

                // Define each variant as a named individual
                let defined_by = self.file_resource_iri(ctx);
                for variant in variants {
                    writeln!(
                        out,
                        "{}:{}_{} rdf:type owl:NamedIndividual , {} ; rdfs:isDefinedBy <{}> .",
                        prefix,
                        concept.name.get(),
                        variant.name,
                        class_ref,
                        defined_by
                    )?;
                }
                writeln!(out)?;
            }
        }

        Ok(())
    }

    /// Write a has declaration as property restriction and property definition.
    fn write_has_property(
        &self,
        out: &mut String,
        has: &HasDeclaration,
        class_ref: &str,
        ctx: &GeneratorContext,
    ) -> Result<(), TurtleError> {
        let prefix = self.namespace_to_prefix(ctx.namespace, ctx.package.namespace());
        let prop_ref = if prefix.is_empty() {
            format!(":{}", has.name)
        } else {
            format!("{}:{}", prefix, has.name)
        };
        let dimension_range = dimension_range_iri_for_ctx(&has.type_ref, ctx);
        let type_ref = match &dimension_range {
            Some(iri) => format!("<{iri}>"),
            None => self.type_ref_to_turtle(&has.type_ref, ctx)?,
        };

        // Determine if it's a datatype or object property. A dimension-typed
        // range (`has weight: unit.Mass`) holds a `quantity(...)` literal, not
        // an instance of the referenced concept, so it is a datatype property
        // even though its `TypeRef` is `Named`.
        let prop_type = match &has.type_ref {
            _ if dimension_range.is_some() => "owl:DatatypeProperty",
            TypeRef::Primitive { .. } => "owl:DatatypeProperty",
            TypeRef::Named { .. } => "owl:ObjectProperty",
        };

        // Write property definition
        writeln!(out, "{} rdf:type {}", prop_ref, prop_type)?;
        writeln!(out, "  ; rdfs:domain {}", class_ref)?;
        writeln!(out, "  ; rdfs:range {}", type_ref)?;

        // Add functional property constraint if cardinality is one or optional
        if matches!(
            has.cardinality,
            Some(Cardinality::One { .. }) | Some(Cardinality::Optional { .. })
        ) {
            writeln!(out, "  ; rdf:type owl:FunctionalProperty")?;
        }

        let skos = self.extract_skos_data(&has.name, has.span, ctx.ontology);
        self.write_skos_triples(out, &skos)?;

        let deferred = self.write_property_axioms(out, &has.axioms, &prop_ref, ctx)?;

        writeln!(out, "  ; rdfs:isDefinedBy <{}>", self.file_resource_iri(ctx))?;
        writeln!(out, "  .")?;

        for triple in &deferred {
            writeln!(out, "{}", triple)?;
        }

        // Write cardinality restriction on the class
        self.write_cardinality_restriction(out, has, class_ref, &prop_ref, &type_ref)?;

        writeln!(out)?;
        Ok(())
    }

    /// Write OWL cardinality restrictions.
    // `type_ref` is unused until the commented `owl:onClass` qualified-cardinality
    // lines below are enabled; keep the param and name for that.
    #[allow(unused_variables)]
    fn write_cardinality_restriction(
        &self,
        out: &mut String,
        has: &HasDeclaration,
        class_ref: &str,
        prop_ref: &str,
        type_ref: &str,
    ) -> Result<(), TurtleError> {
        let card = has.cardinality.as_ref();

        match card {
            Some(Cardinality::One { .. }) => {
                // Exactly one (required single-valued)
                writeln!(out, "{} owl:equivalentClass [", class_ref)?;
                writeln!(out, "  rdf:type owl:Restriction")?;
                writeln!(out, "  ; owl:onProperty {} ;", prop_ref)?;
                writeln!(out, "  owl:cardinality \"1\"^^xsd:nonNegativeInteger")?;
                // writeln!(out, "  owl:onClass {}", type_ref)?; // To be used only if maxQualifiedCardinality is used, and Qualified is used only if domain or range cardinality is express to a sub class (of an union for instance)
                writeln!(out, "  ]\n  .")?;
            }
            Some(Cardinality::Optional { .. }) => {
                // Zero or one
                writeln!(out, "{} owl:equivalentClass [", class_ref)?;
                writeln!(out, "  rdf:type owl:Restriction")?;
                writeln!(out, "  ; owl:onProperty {}", prop_ref)?;
                writeln!(out, "  ; owl:maxCardinality \"1\"^^xsd:nonNegativeInteger")?;
                // writeln!(out, "  owl:onClass {}", type_ref)?; // To be used only if maxQualifiedCardinality is used, and Qualified is used only if domain or range cardinality is express to a sub class (of an union for instance)
                writeln!(out, "  ]\n .")?;
            }
            Some(Cardinality::Any { .. }) | None => {}
            Some(Cardinality::Some { .. }) => {
                // One or more
                writeln!(out, "{} rdfs:subClassOf [", class_ref)?;
                writeln!(out, "  rdf:type owl:Restriction")?;
                writeln!(out, "  ; owl:onProperty {}", prop_ref)?;
                writeln!(out, "  ; owl:minCardinality \"1\"^^xsd:nonNegativeInteger")?;
                // writeln!(out, "  owl:onClass {}", type_ref)?; // To be used only if maxQualifiedCardinality is used, and Qualified is used only if domain or range cardinality is express to a sub class (of an union for instance)
                writeln!(out, "  ]\n .")?;
            }
            Some(Cardinality::Exact { value: n, .. }) => {
                writeln!(out, "{} rdfs:subClassOf [", class_ref)?;
                writeln!(out, "  rdf:type owl:Restriction")?;
                writeln!(out, "  ; owl:onProperty {}", prop_ref)?;
                writeln!(out, "  ; owl:cardinality \"{}\"^^xsd:nonNegativeInteger", n)?;
                // writeln!(out, "  owl:onClass {}", type_ref)?; // To be used only if maxQualifiedCardinality is used, and Qualified is used only if domain or range cardinality is express to a sub class (of an union for instance)
                writeln!(out, "  ]\n .")?;
            }
            Some(Cardinality::Range { min, max, .. }) => {
                writeln!(out, "{} rdfs:subClassOf [", class_ref)?;
                writeln!(out, "  rdf:type owl:Restriction")?;
                writeln!(out, "  ; owl:onProperty {}", prop_ref)?;
                writeln!(
                    out,
                    "  ; owl:minCardinality \"{}\"^^xsd:nonNegativeInteger",
                    min
                )?;
                if let Some(max) = max {
                    writeln!(
                        out,
                        "  ; owl:maxCardinality \"{}\"^^xsd:nonNegativeInteger",
                        max
                    )?;
                }
                // writeln!(out, "  owl:onClass {}", type_ref)?; // To be used only if maxQualifiedCardinality is used, and Qualified is used only if domain or range cardinality is express to a sub class (of an union for instance)
                writeln!(out, "  ]\n .")?;
            }
        }

        Ok(())
    }

    /// Emit `; owl:…` lines for each PropertyAxiom (inline) and collect deferred
    /// complete triples (property chains, `+`, `*`) to be emitted after the `.`.
    fn write_property_axioms(
        &self,
        out: &mut String,
        axioms: &[PropertyAxiom],
        prop_ref: &str,
        ctx: &GeneratorContext,
    ) -> Result<Vec<String>, TurtleError> {
        let mut deferred: Vec<String> = Vec::new();
        for axiom in axioms {
            match axiom {
                PropertyAxiom::Symmetric { .. } => {
                    writeln!(out, "  ; rdf:type owl:SymmetricProperty")?;
                }
                PropertyAxiom::Reflexive { .. } => {
                    writeln!(out, "  ; rdf:type owl:ReflexiveProperty")?;
                }
                PropertyAxiom::Transitive { .. } => {
                    writeln!(out, "  ; rdf:type owl:TransitiveProperty")?;
                }
                PropertyAxiom::Sub { property, .. } => {
                    let target = self.format_qualified_name(property, ctx)?;
                    writeln!(out, "  ; rdfs:subPropertyOf {}", target)?;
                }
                PropertyAxiom::InverseOf { property, .. } => {
                    let target = self.format_qualified_name(property, ctx)?;
                    writeln!(out, "  ; owl:inverseOf {}", target)?;
                }
                PropertyAxiom::EquivalentTo { path, .. } => {
                    let extra = self.write_equivalent_to(out, prop_ref, path, ctx)?;
                    deferred.extend(extra);
                }
            }
        }
        Ok(deferred)
    }

    /// Handle `equivalent to <path>`. Inline-emittable paths write a `; …`
    /// continuation; structural paths (chain, `+`, `*`) return deferred triples.
    fn write_equivalent_to(
        &self,
        out: &mut String,
        prop_ref: &str,
        path: &PropertyPath,
        ctx: &GeneratorContext,
    ) -> Result<Vec<String>, TurtleError> {
        match path {
            PropertyPath::Name { name, .. } => {
                let target = self.format_qualified_name(name, ctx)?;
                writeln!(out, "  ; owl:equivalentProperty {}", target)?;
                Ok(vec![])
            }
            PropertyPath::Inverse { inner, .. } => {
                let inner_str = self.render_property_path(inner, ctx)?;
                writeln!(out, "  ; owl:equivalentProperty [ owl:inverseOf {} ]", inner_str)?;
                Ok(vec![])
            }
            PropertyPath::Alt { .. } => {
                let rendered = self.render_property_path(path, ctx)?;
                writeln!(out, "  ; owl:equivalentProperty {}", rendered)?;
                Ok(vec![])
            }
            PropertyPath::Sequence { steps, .. } => {
                let chain: Result<Vec<_>, _> =
                    steps.iter().map(|s| self.render_property_path(s, ctx)).collect();
                let chain_str = chain?.join(" ");
                let triple = format!(
                    "{} rdfs:subPropertyOf [\n    rdf:type owl:ObjectProperty ;\n    owl:propertyChainAxiom ( {} )\n  ] .",
                    prop_ref, chain_str
                );
                Ok(vec![triple])
            }
            PropertyPath::OneOrMore { inner, .. } => {
                let base = self.render_property_path(inner, ctx)?;
                let mut out_deferred = Vec::new();
                // rdf:type owl:TransitiveProperty goes inline
                writeln!(out, "  ; rdf:type owl:TransitiveProperty")?;
                // base case: prop ⊇ inner
                out_deferred.push(format!("{} rdfs:subPropertyOf {} .", prop_ref, base));
                // chain: prop ⊇ inner∘prop
                out_deferred.push(format!(
                    "{} rdfs:subPropertyOf [\n    rdf:type owl:ObjectProperty ;\n    owl:propertyChainAxiom ( {} {} )\n  ] .",
                    prop_ref, base, prop_ref
                ));
                Ok(out_deferred)
            }
            PropertyPath::ZeroOrMore { inner, .. } => {
                let base = self.render_property_path(inner, ctx)?;
                let mut out_deferred = Vec::new();
                writeln!(out, "  ; rdf:type owl:TransitiveProperty")?;
                writeln!(out, "  ; rdf:type owl:ReflexiveProperty")?;
                out_deferred.push(format!("{} rdfs:subPropertyOf {} .", prop_ref, base));
                out_deferred.push(format!(
                    "{} rdfs:subPropertyOf [\n    rdf:type owl:ObjectProperty ;\n    owl:propertyChainAxiom ( {} {} )\n  ] .",
                    prop_ref, base, prop_ref
                ));
                Ok(out_deferred)
            }
        }
    }

    /// Render a PropertyPath to an inline Turtle string (for chain lists, union lists).
    /// OneOrMore/ZeroOrMore cannot be rendered inline; returns a comment placeholder.
    fn render_property_path(
        &self,
        path: &PropertyPath,
        ctx: &GeneratorContext,
    ) -> Result<String, TurtleError> {
        match path {
            PropertyPath::Name { name, .. } => self.format_qualified_name(name, ctx),
            PropertyPath::Inverse { inner, .. } => {
                let inner_str = self.render_property_path(inner, ctx)?;
                Ok(format!("[ owl:inverseOf {} ]", inner_str))
            }
            PropertyPath::Sequence { steps, .. } => {
                let parts: Result<Vec<_>, _> =
                    steps.iter().map(|s| self.render_property_path(s, ctx)).collect();
                Ok(format!("( {} )", parts?.join(" ")))
            }
            PropertyPath::Alt { left, right, .. } => {
                let l = self.render_property_path(left, ctx)?;
                let r = self.render_property_path(right, ctx)?;
                Ok(format!("[ owl:unionOf ( {} {} ) ]", l, r))
            }
            PropertyPath::OneOrMore { inner, .. } => {
                let name = self.render_property_path(inner, ctx)?;
                Ok(format!("# NOTE: {}+ requires deferred chain axiom", name))
            }
            PropertyPath::ZeroOrMore { inner, .. } => {
                let name = self.render_property_path(inner, ctx)?;
                Ok(format!("# NOTE: {}* requires deferred chain axiom", name))
            }
        }
    }

    /// Write a property definition.
    fn write_property(
        &self,
        out: &mut String,
        property: &PropertyDef,
        ctx: &GeneratorContext,
    ) -> Result<(), TurtleError> {
        let prefix = self.namespace_to_prefix(ctx.namespace, ctx.package.namespace());
        let prop_ref = if prefix.is_empty() {
            format!(":{}", property.name.get())
        } else {
            format!("{}:{}", prefix, property.name.get())
        };

        let domain_ref = self.type_ref_to_turtle(&property.domain, ctx)?;
        let dimension_range = dimension_range_iri_for_ctx(&property.range, ctx);
        let range_ref = match &dimension_range {
            Some(iri) => format!("<{iri}>"),
            None => self.type_ref_to_turtle(&property.range, ctx)?,
        };

        // Determine property type based on range. A dimension-typed range
        // (`unit.Mass`) holds a `quantity(...)` literal, so it's a datatype
        // property even though its `TypeRef` is `Named`.
        let prop_type = match &property.range {
            _ if dimension_range.is_some() => "owl:DatatypeProperty",
            TypeRef::Primitive { .. } => "owl:DatatypeProperty",
            TypeRef::Named { .. } => "owl:ObjectProperty",
        };

        if self.options.include_comments {
            writeln!(out, "# Property: {}", property.name.get())?;
        }

        writeln!(out, "{} rdf:type {}", prop_ref, prop_type)?;

        // Check if functional (domain cardinality is one or optional)
        let is_functional = matches!(
            property.domain_cardinality,
            Some(Cardinality::One { .. }) | Some(Cardinality::Optional { .. }) | None
        );
        if is_functional {
            writeln!(out, "    , owl:FunctionalProperty")?;
        }

        // Check if inverse functional
        let is_inverse_functional = matches!(
            property.range_cardinality,
            Some(Cardinality::One { .. }) | Some(Cardinality::Optional { .. })
        );
        if is_inverse_functional {
            writeln!(out, "    , owl:InverseFunctionalProperty")?;
        }

        writeln!(out, "  ; rdfs:domain {}", domain_ref)?;
        writeln!(out, "  ; rdfs:range {}", range_ref)?;

        let skos = self.extract_skos_data(property.name.get(), property.span, ctx.ontology);
        self.write_skos_triples(out, &skos)?;

        let deferred = self.write_property_axioms(out, &property.axioms, &prop_ref, ctx)?;

        writeln!(out, "  ; rdfs:isDefinedBy <{}>", self.file_resource_iri(ctx))?;
        writeln!(out, "  .")?;

        for triple in &deferred {
            writeln!(out, "{}", triple)?;
        }

        writeln!(out)?;

        Ok(())
    }

    /// Render a `Subject` to its N3 string representation.
    ///
    /// For `Subject::Constraint`, this generates a temp variable (match context)
    /// or blank node (then context) and appends the constraint triples to `out`.
    /// Returns the variable/blank-node name to use in the calling triple.
    fn render_subject(
        &mut self,
        subject: &Subject,
        out: &mut String,
        is_match_context: bool,
        ctx: &GeneratorContext,
    ) -> Result<String, TurtleError> {
        match subject {
            Subject::Variable { name, .. } => Ok(name.to_string()),

            Subject::Constant { name, .. } => self.format_qualified_name(name, ctx),

            Subject::Constraint { block, .. } => {
                let node_name = if is_match_context {
                    self.create_temp_var()
                } else {
                    self.new_blank_node()
                };
                // Emit constraint triples into `out`
                self.write_constraint_triples(out, &node_name, block, is_match_context, ctx)?;
                Ok(node_name)
            }
        }
    }

    /// Emit N3 triples for a constraint block.
    ///
    /// Given a variable/blank-node name and a `ConstraintBlock`, emits
    /// triples depending on the constraint variant:
    /// - `TypeIs`            → `?_v1 a :SomeType .`
    /// - `Comparison`        → `?_v1 math:greaterThan 18 .`
    /// - `PropertyValue`     → `?_v1 :prop ?val .`
    /// - `PropertyConstraint`→ `?_v1 :prop ?_v2 .` + recursive constraint triples on `?_v2`
    fn write_constraint_triples(
        &mut self,
        out: &mut String,
        node_name: &str,
        block: &ConstraintBlock,
        is_match_context: bool,
        ctx: &GeneratorContext,
    ) -> Result<(), TurtleError> {
        for constraint in &block.constraints {
            match constraint {
                Constraint::TypeIs { type_ref, .. } => {
                    let type_str = self.type_ref_to_turtle(type_ref, ctx)?;
                    write!(out, "{} a {} . ", node_name, type_str)?;
                }

                Constraint::Comparison {
                    operator, value, ..
                } => {
                    let predicate = self.comparison_predicate(operator, value);
                    let value_str = self.render_expr(out, value, is_match_context);
                    write!(out, "{} {} {} . ", node_name, predicate, value_str)?;
                }

                Constraint::PropertyValue {
                    property, value, ..
                } => {
                    let prop_str = self.format_qualified_name(property, ctx)?;
                    let mut follow = String::new();
                    let value_str =
                        self.render_object(value, &mut follow, is_match_context, ctx)?;
                    write!(out, "{} {} {} . ", node_name, prop_str, value_str)?;
                    writeln!(out, "{}", follow)?;
                }

                Constraint::PropertyConstraint {
                    property,
                    block: nested_block,
                    ..
                } => {
                    // Create an intermediate node for the nested constraint
                    let inner_node = if is_match_context {
                        self.create_temp_var()
                    } else {
                        self.new_blank_node()
                    };

                    // Link the current node to the inner node via the property
                    let prop_str = self.format_qualified_name(property, ctx)?;
                    write!(out, "{} {} {} . ", node_name, prop_str, inner_node)?;

                    // Recursively emit the nested constraint triples on the inner node
                    self.write_constraint_triples(
                        out,
                        &inner_node,
                        nested_block,
                        is_match_context,
                        ctx,
                    )?;
                }

                // `is prop of value` → inverse direction: `value prop node_name`.
                Constraint::Inverse { property, value, .. } => {
                    let prop_str = self.format_qualified_name(property, ctx)?;
                    let mut follow = String::new();
                    let value_str =
                        self.render_object(value, &mut follow, is_match_context, ctx)?;
                    write!(out, "{} {} {} . ", value_str, prop_str, node_name)?;
                    writeln!(out, "{}", follow)?;
                }

                // `is prop of [ ... ]` → `inner prop node_name`, constraints on `inner`.
                Constraint::InverseNested {
                    property,
                    block: nested_block,
                    ..
                } => {
                    let inner_node = if is_match_context {
                        self.create_temp_var()
                    } else {
                        self.new_blank_node()
                    };
                    let prop_str = self.format_qualified_name(property, ctx)?;
                    write!(out, "{} {} {} . ", inner_node, prop_str, node_name)?;
                    self.write_constraint_triples(
                        out,
                        &inner_node,
                        nested_block,
                        is_match_context,
                        ctx,
                    )?;
                }
            }
        }
        Ok(())
    }

    /// Choose the comparison predicate. When the compared-against value is a
    /// physical-quantity literal, emit a unit-aware `dq:` predicate (resolved by
    /// retox against the unit datatype); otherwise the standard `math:` builtin.
    fn comparison_predicate(&self, op: &ComparisonOp, value: &Expr) -> &'static str {
        let is_quantity = matches!(
            value,
            Expr::Literal { value: Literal::Quantity { .. }, .. }
        );
        if !is_quantity {
            return self.comparison_op_to_n3(op);
        }
        match op {
            ComparisonOp::Equal => "dq:equalTo",
            ComparisonOp::NotEqual => "dq:notEqualTo",
            ComparisonOp::LessThan => "dq:lessThan",
            ComparisonOp::LessEqual => "dq:lessThanOrEqual",
            ComparisonOp::GreaterThan => "dq:greaterThan",
            ComparisonOp::GreaterEqual => "dq:greaterThanOrEqual",
        }
    }

    /// Map a `ComparisonOp` to its N3/math predicate.
    fn comparison_op_to_n3(&self, op: &ComparisonOp) -> &'static str {
        match op {
            ComparisonOp::Equal => "math:equalTo",
            ComparisonOp::NotEqual => "math:notEqualTo",
            ComparisonOp::LessThan => "math:lessThan",
            ComparisonOp::LessEqual => "math:notGreaterThan",
            ComparisonOp::GreaterThan => "math:greaterThan",
            ComparisonOp::GreaterEqual => "math:notLessThan",
        }
    }

    /// Create a fresh temporary variable name for match-context constraints.
    /// Returns something like "?_v1", "?_v2", etc.
    fn create_temp_var(&mut self) -> String {
        // Assumed to exist on self, incrementing a counter
        self.var_counter += 1;
        format!("?_v{}", self.var_counter)
    }

    /// Create a fresh blank node identifier for then-context constraints.
    /// Returns something like "_:b1", "_:b2", etc.
    fn new_blank_node(&mut self) -> String {
        // Assumed to exist on self, incrementing a counter
        self.blank_counter += 1;
        format!("_:b{}", self.blank_counter)
    }

    /// Render an `Object` to its N3 string representation.
    ///
    /// Similar to `render_subject`, constraints generate auxiliary triples.
    fn render_object(
        &mut self,
        object: &Object,
        out: &mut String,
        is_match_context: bool,
        ctx: &GeneratorContext,
    ) -> Result<String, TurtleError> {
        match object {
            Object::Variable { name, .. } => Ok(name.to_string()),

            Object::Constant { value, .. } => self.format_qualified_name(value, ctx),

            Object::Literal { value, .. } => Ok(self.render_expr(out, value, is_match_context)),

            Object::Constraint { block } => {
                let node_name = if is_match_context {
                    self.create_temp_var()
                } else {
                    self.new_blank_node()
                };
                self.write_constraint_triples(out, &node_name, block, is_match_context, ctx)?;
                Ok(node_name)
            }
        }
    }

    /// Render a `Literal` in N3/Turtle syntax.
    /// Declare the `dq:` prefix (only called when a quantity was actually
    /// emitted). Unit datatypes are written as full IRIs, so no `unit:` prefix
    /// is needed.
    fn write_quantity_prefixes(&self, out: &mut String) -> Result<(), TurtleError> {
        writeln!(out, "@prefix dq: <{}> .", DQ_IRI_BASE)?;
        Ok(())
    }

    /// Emit the once-per-unit definition block for every quantity unit rendered
    /// in this output: `<unit-iri> dq:coefficient <c> ; dq:dimension "<dim>" ;
    /// dq:symbol "<sym>" .` The subject is the same full unit IRI used as the
    /// value datatype, so a SPARQL `datatype(?q)` join lands on it.
    fn write_unit_definitions(&self, out: &mut String) -> Result<(), TurtleError> {
        let defs = self.unit_defs.borrow();
        if defs.is_empty() {
            return Ok(());
        }
        writeln!(out)?;
        writeln!(out, "# Quantity unit definitions")?;
        for (canon, (coef, dim, symbol)) in defs.iter() {
            writeln!(
                out,
                "<{}{}> dq:coefficient {} ; dq:dimension \"{}\" ; dq:symbol \"{}\" .",
                dolfin_units::UNIT_IRI_BASE,
                canon,
                coef,
                dim,
                escape_turtle_string(symbol),
            )?;
        }
        Ok(())
    }

    fn render_literal(&self, literal: &Literal) -> String {
        match literal {
            Literal::Int { value: v, .. } => format!("\"{}\"^^xsd:integer", v),
            Literal::Float { value: v, .. } => format!("\"{}\"^^xsd:float", v),
            Literal::String { value: v, .. } => format!("\"{}\"", v.replace('"', "\\\"")),
            Literal::Boolean { value: v, .. } => format!("\"{}\"^^xsd:boolean", v),
            Literal::Iri { value: v, .. } => format!("<{}>", v),
            Literal::Temporal { content, .. } => match literal.resolve_temporal(&self.temporal_ctx) {
                Some(Ok((value, xsd_type))) => format!("\"{}\"^^{}", value, xsd_type),
                // FIXME: this swallows two error cases that should be author
                // diagnostics — a numeric date with no inline mask (needs
                // file-level @locale, not yet plumbed) and a kind mismatch such
                // as date(7d). `render_literal` returns `String` with no error
                // channel, so for now we emit a plain string to keep the Turtle
                // valid. Thread diagnostics through here with the @locale work.
                _ => format!("\"{}\"", content.replace('"', "\\\"")),
            },
            // Representation "L" (see dolfin-units-turtle-plan.md): the value as
            // written in the lexical slot, the unit carried in a canonical
            // `unit:` datatype IRI. The unit's coefficient/dimension/symbol are
            // recorded in `unit_defs` and emitted once in a definition block.
            // A dimensionless quantity carries no unit, so it degrades to a
            // plain `xsd:double`. On parse error, fall back to a plain string
            // (the dolfin-analysis pass reports the diagnostic).
            Literal::Quantity { content, .. } => match literal.resolve_quantity_with(&self.unit_registry.borrow()) {
                Some(Ok(q)) => {
                    let canon = q.canonical_unit();
                    if canon == "1" {
                        format!("\"{}\"^^xsd:double", q.qty)
                    } else {
                        self.unit_defs.borrow_mut().entry(canon.clone()).or_insert_with(|| {
                            (q.scale, q.dimensions.canonical_string(), q.display_unit.clone())
                        });
                        // Full IRI (not a `unit:` prefixed name): the canonical
                        // local part contains `.`/`-`, which the retox N3 lexer
                        // rejects in a PN_LOCAL. A full IRI is accepted by both
                        // retox and oxigraph, and `datatype()` resolves to the
                        // same IRI regardless.
                        format!("\"{}\"^^<{}{}>", q.qty, dolfin_units::UNIT_IRI_BASE, canon)
                    }
                }
                _ => format!("\"{}\"", content.replace('"', "\\\"")),
            },
        }
    }

    /// Render an `Expr`, emitting intermediate math triples into `out` and
    /// returning the Turtle term (literal or blank node) that holds the result.
    fn render_expr(&mut self, out: &mut String, expr: &Expr, is_match_context: bool) -> String {
        match expr {
            Expr::Literal { value, .. } => self.render_literal(value),
            Expr::Variable { name, .. } => name.clone(),
            Expr::BinaryOp {
                op, left, right, ..
            } => {
                let predicate = match op {
                    BinaryOp::Add => "math:sum",
                    BinaryOp::Sub => "math:difference",
                    BinaryOp::Mul => "math:product",
                    BinaryOp::Div => "math:quotient",
                };
                // `math:sum` / `math:product` accept an N-element list, and `+`/`*`
                // are associative, so flatten a same-op chain into a single triple.
                // `math:difference` / `math:quotient` are strictly binary and
                // non-associative, so keep them pairwise.
                let operands: Vec<&Expr> = match op {
                    BinaryOp::Add | BinaryOp::Mul => {
                        let mut acc = Vec::new();
                        collect_chain(op, expr, &mut acc);
                        acc
                    }
                    BinaryOp::Sub | BinaryOp::Div => vec![left.as_ref(), right.as_ref()],
                };
                let refs: Vec<String> = operands
                    .iter()
                    .map(|e| self.render_expr(out, e, is_match_context))
                    .collect();
                let result = if is_match_context {
                    self.create_temp_var()
                } else {
                    self.new_blank_node()
                };
                writeln!(out, "( {} ) {} {} .", refs.join(" "), predicate, result).unwrap();
                result
            }
            Expr::UnaryOp { op, operand, .. } => {
                let operand_ref = self.render_expr(out, operand, is_match_context);
                let result = if is_match_context {
                    self.create_temp_var()
                } else {
                    self.new_blank_node()
                };
                match op {
                    UnaryOp::Neg => {
                        writeln!(out, "{} math:negation {} .", operand_ref, result).unwrap();
                    }
                }
                result
            }
        }
    }

    /// Write a fact (named individual + property assertions) as Turtle triples.
    ///
    /// Per spec §9:
    /// - `fact rex a Dog`  → `:rex rdf:type owl:NamedIndividual , :Dog .`
    /// - `weight 45.0`     → `:rex :weight "45"^^xsd:double .`
    /// - `:John` reference → `:John`
    /// - `[ ... ]` block   → fresh blank node with embedded triples
    /// - `is P of V`       → `<V> :P :rex .`  (inverse direction)
    fn write_fact(
        &mut self,
        out: &mut String,
        fact: &FactDef,
        ctx: &GeneratorContext,
    ) -> Result<(), TurtleError> {
        let prefix = self.namespace_to_prefix(ctx.namespace, ctx.package.namespace());
        let instance_ref = if prefix.is_empty() {
            format!(":{}", fact.id)
        } else {
            format!("{}:{}", prefix, fact.id)
        };

        if self.options.include_comments {
            writeln!(out, "# Fact: {}", fact.id)?;
        }

        // rdf:type declaration — always include owl:NamedIndividual
        write!(out, "{} rdf:type owl:NamedIndividual", instance_ref)?;
        for type_qn in &fact.types {
            let type_str = self.format_qualified_name(type_qn, ctx)?;
            write!(out, "\n  , {}", type_str)?;
        }
        writeln!(out, "\n  ; rdfs:isDefinedBy <{}>", self.file_resource_iri(ctx))?;
        writeln!(out, "  .")?;

        // Property and inverse assertions
        for assertion in &fact.assertions {
            match assertion {
                FactAssertion::Property { property, values, .. } => {
                    let prop_ref = self.format_qualified_name(property, ctx)?;
                    for value in values {
                        let value_str = self.fact_value_to_turtle(out, value, ctx)?;
                        writeln!(out, "{} {} {} .", instance_ref, prop_ref, value_str)?;
                    }
                }
                FactAssertion::Inverse { property, value, .. } => {
                    // `is P of V` → V P instance (the other entity holds the predicate)
                    let prop_str = self.format_qualified_name(property, ctx)?;
                    let value_str = self.fact_value_to_turtle(out, value, ctx)?;
                    writeln!(out, "{} {} {} .", value_str, prop_str, instance_ref)?;
                }
                FactAssertion::TypeHint { .. } => {
                    // Type hints are only meaningful inside anonymous blocks
                }
            }
        }
        writeln!(out)?;
        Ok(())
    }

    /// Render a `FactValue` to a Turtle term string.
    ///
    /// For anonymous blocks a fresh blank node is minted and the block's
    /// assertions are emitted into `out` before returning the blank-node label.
    fn fact_value_to_turtle(
        &mut self,
        out: &mut String,
        value: &FactValue,
        ctx: &GeneratorContext,
    ) -> Result<String, TurtleError> {
        match value {
            FactValue::Literal { value: lit, .. } => Ok(self.render_literal(lit)),

            FactValue::Reference { qualifier: None, name, .. } => Ok(format!(":{}", name)),

            FactValue::Reference { qualifier: Some(q), name, .. } => {
                Ok(format!("{}:{}", q, name))
            }

            FactValue::Named { name, .. } => self.format_qualified_name(name, ctx),

            FactValue::Block { assertions, .. } => {
                let blank = self.new_blank_node();

                for assertion in assertions {
                    match assertion {
                        FactAssertion::TypeHint { type_ref, .. } => {
                            let type_str = self.format_qualified_name(type_ref, ctx)?;
                            writeln!(out, "{} rdf:type {} .", blank, type_str)?;
                        }
                        FactAssertion::Property { property, values, .. } => {
                            let prop_ref = self.format_qualified_name(property, ctx)?;
                            for v in values {
                                let v_str = self.fact_value_to_turtle(out, v, ctx)?;
                                writeln!(out, "{} {} {} .", blank, prop_ref, v_str)?;
                            }
                        }
                        FactAssertion::Inverse { property, value, .. } => {
                            let prop_str = self.format_qualified_name(property, ctx)?;
                            let v_str = self.fact_value_to_turtle(out, value, ctx)?;
                            writeln!(out, "{} {} {} .", v_str, prop_str, blank)?;
                        }
                    }
                }
                Ok(blank)
            }
        }
    }

    /// Write a query transpiled to SPARQL as comments.
    fn write_query(
        &self,
        out: &mut String,
        query: &rowl::ast::QueryDef,
        ctx: &GeneratorContext,
    ) -> Result<(), TurtleError> {
        let all_queries: Vec<rowl::ast::QueryDef> = ctx.ontology.ast.queries();
        let refs: Vec<&rowl::ast::QueryDef> = all_queries.iter().collect();
        writeln!(out, "# Query: {}", query.name)?;
        match to_sparql(query, &refs) {
            Ok(sparql) => {
                for line in sparql.lines() {
                    writeln!(out, "# {}", line)?;
                }
            }
            Err(e) => {
                writeln!(out, "# (could not transpile to SPARQL: {})", e)?;
            }
        }
        writeln!(out)?;
        Ok(())
    }

    /// Write a rule as a comment.
    fn write_rule(
        &mut self,
        out: &mut String,
        rule: &RuleDef,
        ctx: &GeneratorContext,
    ) -> Result<(), TurtleError> {
        // 1. Flatten the rule tree into a list of flat rules
        let flat_rules = flatten_rule(rule, &[]);

        // 2. Emit each flat rule
        for flat in &flat_rules {
            self.write_flat_rule(out, flat, ctx)?;
        }
        Ok(())
        // if self.options.include_rules_as_comments {
        //     writeln!(out, "# Rule: {}", rule.name)?;
        //     writeln!(
        //         out,
        //         "#   (Rules are exported as comments; SWRL support planned)"
        //     )?;
        //     for pattern in &rule.match_block.patterns {
        //         self.write_pattern_in_match(out, pattern, _namespace)?;
        //     }
        //     writeln!(out, "#   then: {} items", rule.then_block.items.len())?;
        //     writeln!(out)?;
        // }
        // Ok(())
    }

    /// Write a single flattened rule in N3 syntax.
    fn write_flat_rule(
        &mut self,
        out: &mut String,
        flat: &FlatN3Rule,
        ctx: &GeneratorContext,
    ) -> Result<(), TurtleError> {
        // Build the match body (antecedent)
        let mut match_body = String::new();
        for pattern in &flat.match_patterns {
            self.write_pattern(&mut match_body, pattern, true, ctx)?;
        }

        // Build the then body (consequent)
        let mut then_body = String::new();
        for item in &flat.then_items {
            self.write_then_item(&mut then_body, item, ctx)?;
        }

        // Render the whole rule into a temporary string, uncommented, then —
        // if `include_rules_as_comments` is set — prefix every line with "# "
        // in one pass, rather than threading the comment marker through each
        // pattern/then-item renderer.
        let mut rule = String::new();
        writeln!(rule, "Rule: {}", flat.name)?;
        writeln!(
            rule,
            "{{\n{}\n}}  => {{\n{}\n}} .",
            match_body.trim_end(),
            then_body.trim_end(),
        )?;

        if self.options.include_rules_as_comments {
            for line in rule.lines() {
                writeln!(out, "# {}", line)?;
            }
        } else {
            write!(out, "{}", rule)?;
        }
        writeln!(out)?;

        Ok(())
    }

    /// Render a single match pattern as N3 triples.
    ///
    /// `is_match_context` = true means we're in a match block (constraints
    /// generate `?_v` temp variables); false means then block (blank nodes).
    fn write_pattern(
        &mut self,
        out: &mut String,
        pattern: &Pattern,
        is_match_context: bool,
        ctx: &GeneratorContext,
    ) -> Result<(), TurtleError> {
        match pattern {
            Pattern::Triple {
                subject,
                property,
                object,
                span: _,
            } => {
                let mut precede = String::new();
                let subj_str = self.render_subject(subject, &mut precede, is_match_context, ctx)?;
                let prop_str = self.format_qualified_name(property, ctx)?;
                let mut follow = String::new();
                let obj_str = self.render_object(object, &mut follow, is_match_context, ctx)?;
                write!(out, "  {} {} {} . ", subj_str, prop_str, obj_str)?;
                writeln!(out, "{} {}", precede, follow)?;
            }

            Pattern::Type {
                subject,
                type_ref,
                span: _,
            } => {
                let subj_str = self.render_subject(subject, out, is_match_context, ctx)?;
                let type_str = self.type_ref_to_turtle(type_ref, ctx)?;
                writeln!(out, "  {} a {} . ", subj_str, type_str)?;
            }

            Pattern::Quantified {
                quantifier,
                variable: _,
                constraint: _,
                patterns: _,
                span: _,
            } => {
                // Quantifiers are not yet supported in N3 output
                writeln!(out, "# Quantifier ({}) present, not rendered", quantifier)?;
            }

            Pattern::QueryCall { name, .. } => {
                writeln!(out, "# QueryCall ({}) not rendered in N3 output", name)?;
            }

            Pattern::Inverse {
                subject,
                property,
                object,
                span: _,
            } => {
                // `subject is property of object` desugars to `object property subject`.
                let mut follow = String::new();
                let obj_str = self.render_object(object, &mut follow, is_match_context, ctx)?;
                let prop_str = self.format_qualified_name(property, ctx)?;
                let mut precede = String::new();
                let subj_str =
                    self.render_subject(subject, &mut precede, is_match_context, ctx)?;
                write!(out, "  {} {} {} . ", obj_str, prop_str, subj_str)?;
                writeln!(out, "{} {}", precede, follow)?;
            }
        }
        Ok(())
    }

    /// Render a single then item as N3 triples.
    fn write_then_item(
        &mut self,
        out: &mut String,
        item: &ThenItem,
        ctx: &GeneratorContext,
    ) -> Result<(), TurtleError> {
        match item {
            ThenItem::AssertionTriple { assertion, .. } => {
                let subj_str = self.render_subject(&assertion.subject, out, false, ctx)?;
                let prop_str = self.format_qualified_name(&assertion.property, ctx)?;
                let obj_str = self.render_object(&assertion.object, out, false, ctx)?;
                writeln!(out, "  {} {} {} . ", subj_str, prop_str, obj_str)?;
            }

            ThenItem::AssertionTyping {
                subject, typing, ..
            } => {
                let subj_str = self.render_subject(subject, out, false, ctx)?;
                let type_str = self.format_qualified_name(typing, ctx)?;
                writeln!(out, "  {} a {} . ", subj_str, type_str)?;
            }

            ThenItem::NestedRule { .. } => {
                // Should never reach here — nested rules are extracted during flattening
                unreachable!("Nested rules should have been flattened before emission");
            }
        }
        Ok(())
    }

    fn format_qualified_name(
        &self,
        name: &QualifiedName,
        ctx: &GeneratorContext,
    ) -> Result<String, TurtleError> {
        if name.is_prefixed {
            // `alias:Local` syntax — alias is the exact Turtle prefix, emit directly
            let alias = &name.parts[0];
            let local = name.parts[1..].join("_");
            if ctx.ontology.resolved_prefixes.contains_key(alias) {
                Ok(format!("{}:{}", alias, local))
            } else {
                Err(TurtleError::UnresolvedType(format!("{}:{}", alias, local)))
            }
        } else if name.parts.len() == 1 {
            // Local name - prefix with current namespace
            let prefix = self.namespace_to_prefix(ctx.namespace, ctx.package.namespace());
            if prefix.is_empty() {
                Ok(format!(":{}", name.parts[0]))
            } else {
                Ok(format!("{}:{}", prefix, name.parts[0]))
            }
        } else if name.parts.len() >= 2 {
            // Dot-separated: check if first part is a known package-namespace alias
            let maybe_alias = &name.parts[0];
            if let Some(resolved_ns) = ctx.ontology.resolved_prefixes.get(maybe_alias) {
                let prefix = self.namespace_to_prefix(resolved_ns, ctx.package.namespace());
                let local_name = name.parts[1..].join("_");
                Ok(format!("{}:{}", prefix, local_name))
            } else {
                // Treat as fully qualified namespace path
                let prefix = self.namespace_to_prefix(name, ctx.package.namespace());
                let local_name = name.last();
                if prefix.is_empty() {
                    Ok(format!(":{}", local_name))
                } else {
                    Ok(format!("{}:{}", prefix, local_name))
                }
            }
        } else {
            Err(TurtleError::UnresolvedType(format!("[{}]", name.full())))
        }
    }

    /// Convert a type reference to Turtle syntax.
    fn type_ref_to_turtle(
        &self,
        type_ref: &TypeRef,
        ctx: &GeneratorContext,
    ) -> Result<String, TurtleError> {
        match type_ref {
            TypeRef::Primitive { kind, .. } => Ok(self.primitive_to_xsd(kind).to_string()),
            TypeRef::Named { name, .. } => self.format_qualified_name(name, ctx),
        }
    }

    /// Convert a primitive kind to XSD datatype.
    fn primitive_to_xsd(&self, kind: &PrimitiveKind) -> &'static str {
        match kind {
            PrimitiveKind::String => "xsd:string",
            PrimitiveKind::Int => "xsd:integer",
            PrimitiveKind::Float => "xsd:double",
            PrimitiveKind::Boolean => "xsd:boolean",
            PrimitiveKind::Date => "xsd:date",
            PrimitiveKind::DateTime => "xsd:dateTime",
            PrimitiveKind::Time => "xsd:time",
            PrimitiveKind::Duration => "xsd:duration",
        }
    }

    /// Convert a namespace to a Turtle prefix name.
    fn namespace_to_prefix(
        &self,
        namespace: &QualifiedName,
        base_namespace: &QualifiedName,
    ) -> String {
        // If it's the base namespace, use empty prefix
        if namespace == base_namespace {
            return String::new();
        }

        // If it starts with base namespace, use relative path
        if namespace.parts.len() > base_namespace.parts.len()
            && namespace.parts[..base_namespace.parts.len()] == base_namespace.parts[..]
        {
            let relative: Vec<_> = namespace.parts[base_namespace.parts.len()..].to_vec();
            return relative.join("_");
        }

        // External namespace - use full name with underscores
        namespace.parts.join("_")
    }

    /// Convert a namespace to an IRI.
    ///
    /// If the namespace is already rooted at an absolute IRI — e.g. the package
    /// was declared `package <http://example.com/Foo>:`, so `parts[0]` is the
    /// full IRI — the parts already form the complete IRI and prepending
    /// `base_iri` would double it (`{base}{base}/...`). In that case skip the
    /// prefix.
    fn namespace_to_iri(&self, namespace: &QualifiedName) -> String {
        namespace_iri(&self.options.base_iri, namespace, None)
    }

    /// The file-resource IRI for the current file: its namespace IRI, honoring
    /// a header-level `@iri_name` override when the file has one, with no
    /// trailing terminator.
    ///
    /// Every entity in the file carries `rdfs:isDefinedBy <this>`. When the
    /// file has no `@iri_name`, `<this>` is the canonical path-derived IRI and
    /// (for a non-root file) itself carries `rdfs:isDefinedBy <package-iri>` —
    /// a single hop from entity to package. When the file *does* have an
    /// `@iri_name`, `<this>` is the override IRI, and `write_ontology_file`
    /// additionally emits `<override> rdfs:isDefinedBy <canonical>` so the
    /// reverse direction (mekarui) can still walk override → canonical →
    /// package to recover the file's on-disk path and detect that an override
    /// was used, even though entities now point straight at the override.
    fn file_resource_iri(&self, ctx: &GeneratorContext) -> String {
        self.namespace_to_iri_with_override(ctx.namespace, ctx.ontology.iri_name.as_ref())
    }

    /// Convert a namespace to an IRI with optional name override.
    fn namespace_to_iri_with_override(
        &self,
        namespace: &QualifiedName,
        iri_name_override: Option<&IriNameValue>,
    ) -> String {
        namespace_iri(&self.options.base_iri, namespace, iri_name_override)
    }
}

/// Compute the IRI for an ontology namespace.
///
/// This is the single source of truth for `namespace → IRI`: Turtle data
/// generation and every downstream consumer (SparNatural config, the SaaS IR)
/// must call this so the IRIs they emit are identical to the seeded data.
///
/// - If the package was declared with an absolute IRI (`package <http://…>:`),
///   `parts[0]` already contains the scheme; `base_iri` is not prepended.
/// - An `@iri_name` override replaces the trailing segment (`LocalSegment`) or
///   the whole IRI (`AbsoluteUri`).
pub fn namespace_iri(
    base_iri: &str,
    namespace: &QualifiedName,
    iri_name_override: Option<&IriNameValue>,
) -> String {
    match iri_name_override {
        Some(IriNameValue::AbsoluteUri(uri)) => uri.clone(),
        Some(IriNameValue::LocalSegment(name)) => {
            if namespace.parts.len() > 1 {
                let prefix = &namespace.parts[..namespace.parts.len() - 1];
                if prefix.first().is_some_and(|p| p.contains("://")) {
                    format!("{}/{}", join_parts(prefix), name)
                } else {
                    format!("{}/{}/{}", strip_terminator(base_iri), prefix.join("/"), name)
                }
            } else if namespace.parts.first().is_some_and(|p| p.contains("://")) {
                format!("{}/{}", join_parts(&namespace.parts), name)
            } else {
                format!("{}/{}", strip_terminator(base_iri), name)
            }
        }
        None => {
            if namespace.parts.first().is_some_and(|p| p.contains("://")) {
                join_parts(&namespace.parts)
            } else {
                format!("{}/{}", strip_terminator(base_iri), namespace.parts.join("/"))
            }
        }
    }
}

/// Strip a single trailing namespace terminator (`#` or `/`) from an IRI.
///
/// Ontology namespaces in the suite use a `#` fragment terminator, which the
/// callers of [`namespace_iri`] append themselves. So this helper returns the
/// bare stem: a package declared `<http://demo/test#>` or `<http://demo/test/>`
/// both collapse to `http://demo/test`, which then re-terminates cleanly.
fn strip_terminator(s: &str) -> &str {
    match s.strip_suffix('#') {
        Some(stem) => stem,
        None => s.strip_suffix('/').unwrap_or(s),
    }
}

/// Join namespace parts with a single `/`, stripping any terminator the first
/// part carries. Only `parts[0]` can hold an absolute IRI (with a trailing `/`
/// or `#`); the rest are plain segment names. This is what prevents a
/// slash-declared base from producing `//` and a hash-declared base from
/// producing `#/` at the join boundary.
fn join_parts(parts: &[String]) -> String {
    let mut out = String::new();
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            out.push_str(strip_terminator(part));
        } else {
            out.push('/');
            out.push_str(part);
        }
    }
    out
}

/// Validate a `#xx>` / `definition@xx` language tag against the shared charset
/// `^[a-z]{2,3}(-[A-Za-z0-9]+)*$` (lowercase 2-3 char primary subtag + optional
/// `-SUBTAG` segments, e.g. `en`, `pt-BR`, `zh-Hant`).
fn is_lang_tag(s: &str) -> bool {
    rowl::comment::lang_tag::is_lang_tag(s)
}

/// Split a trimmed comment line into an optional language tag and its text.
///
/// Recognises the leading `#xx> text` prefix: on a trimmed `line`, if it matches
/// `^([a-z]{2,3}(-[A-Za-z0-9]+)*)>( .*)?$` returns `(Some(lang), text)` with at
/// most one leading space after `>` stripped and internal spacing preserved;
/// otherwise `(None, line.to_string())`. The `>` must be followed by a single
/// space or end-of-line, so `# TODO> x`, `# a>b`, `# note: a > b` are untagged.
fn parse_lang_prefix(line: &str) -> (Option<String>, String) {
    rowl::comment::lang_tag::parse_lang_prefix(line)
}

/// Scan leading comments for a `#@ glossary` annotation and description groups.
///
/// Returns `(comment_groups, annotation)`. Description lines are grouped by
/// language via [`parse_lang_prefix`]: consecutive lines sharing a language form
/// one group (joined with a single space), preserving first-appearance order.
/// Each group becomes one `rdfs:comment` literal. Handles merged comment tokens
/// where a regular description comment and a `#@ glossary` block appear on
/// adjacent lines and were merged by the lexer into a single `Comment`.
fn parse_leading_for_glossary(
    leading: &[rowl::comment::Comment],
) -> (
    Vec<(Option<String>, String)>,
    Option<rowl::annotation::ParsedAnnotation>,
) {
    // Append a trimmed description line to `groups`, joining it into the
    // previous group when both share the same language, else starting a new one.
    fn push_desc(groups: &mut Vec<(Option<String>, String)>, trimmed: &str) {
        if trimmed.is_empty() {
            return;
        }
        let (lang, text) = parse_lang_prefix(trimmed);
        match groups.last_mut() {
            Some((prev_lang, prev_text)) if *prev_lang == lang => {
                prev_text.push(' ');
                prev_text.push_str(&text);
            }
            _ => groups.push((lang, text)),
        }
    }

    let mut comment_groups: Vec<(Option<String>, String)> = Vec::new();
    let mut ann_lines: Vec<String> = Vec::new();

    for comment in leading {
        // `comment.text` is everything after the leading `#`.
        // An annotation line starts with `@` (after trimming); every other
        // non-empty line is a description line (grouped by `#xx>` language).
        for line in comment.text.split('\n') {
            let trimmed = line.trim();
            if trimmed.starts_with('@') {
                ann_lines.push(trimmed.to_string());
            } else {
                push_desc(&mut comment_groups, trimmed);
            }
        }
    }

    let annotation = if ann_lines.is_empty() {
        None
    } else {
        // Re-assemble as a fake `#@` comment and parse it with the standard parser.
        let ann_text = ann_lines.join("\n");
        let fake = RawComment {
            text: ann_text,
            raw: String::new(),
            span: RawSpan {
                start: Location::default(),
                end: Location::default(),
            },
            line: 0,
            column: 0,
        };
        parse_annotation(&fake).filter(|a| a.name == "glossary")
    };

    (comment_groups, annotation)
}

/// Recursively flatten a rule tree.
///
/// `parent_patterns` accumulates match patterns from ancestor rules.
/// Each nested rule inherits all ancestor match patterns plus its own.
fn flatten_rule(rule: &RuleDef, parent_patterns: &[Pattern]) -> Vec<FlatN3Rule> {
    let mut results = Vec::new();

    // Combined match patterns: parent's + this rule's own
    let mut combined_patterns: Vec<Pattern> = parent_patterns.to_vec();
    combined_patterns.extend(rule.match_block.patterns.clone());

    // Separate then items into simple assertions and nested rules
    let mut simple_items: Vec<ThenItem> = Vec::new();
    let mut nested_rules: Vec<RuleDef> = Vec::new();

    for item in &rule.then_block.items {
        match item {
            ThenItem::NestedRule { rule: nested } => {
                nested_rules.push(nested.clone());
            }
            other => {
                simple_items.push(other.clone());
            }
        }
    }

    // Emit the current rule (only if it has non-nested then items)
    if !simple_items.is_empty() {
        results.push(FlatN3Rule {
            name: rule.name.clone(),
            match_patterns: combined_patterns.clone(),
            then_items: simple_items,
        });
    }

    // Recursively flatten each nested rule, passing combined patterns down
    for (i, nested) in nested_rules.iter().enumerate() {
        // Give nested rules a derived name: "Parent-1", "Parent-2", etc.
        let nested_with_name = RuleDef {
            name: format!("{}-{}", rule.name, i + 1),
            match_block: nested.match_block.clone(),
            then_block: nested.then_block.clone(),
            span: None,
        };
        let sub_results = flatten_rule(&nested_with_name, &combined_patterns);
        results.extend(sub_results);
    }

    results
}

/// Escape a string for Turtle string literals.
fn escape_turtle_string(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_primitive_to_xsd() {
        let gener = TurtleGenerator::with_defaults();
        assert_eq!(gener.primitive_to_xsd(&PrimitiveKind::String), "xsd:string");
        assert_eq!(gener.primitive_to_xsd(&PrimitiveKind::Int), "xsd:integer");
        assert_eq!(gener.primitive_to_xsd(&PrimitiveKind::Float), "xsd:double");
        assert_eq!(
            gener.primitive_to_xsd(&PrimitiveKind::Boolean),
            "xsd:boolean"
        );
    }

    #[test]
    fn test_dimension_range_iri() {
        let mass = TypeRef::Named {
            name: QualifiedName::new(vec!["unit".into(), "Mass".into()], None),
            span: None,
        };
        assert_eq!(
            dimension_range_iri(&mass),
            Some("https://dolfin.dev/dimension/M1".to_string())
        );

        let ordinary = TypeRef::Named {
            name: QualifiedName::new(vec!["Organization".into()], None),
            span: None,
        };
        assert_eq!(dimension_range_iri(&ordinary), None);

        let primitive = TypeRef::Primitive { kind: PrimitiveKind::Float, span: None };
        assert_eq!(dimension_range_iri(&primitive), None);
    }

    #[test]
    fn test_namespace_to_iri() {
        let gener = TurtleGenerator::new(TurtleOptions {
            base_iri: "http://example.org/".to_string(),
            ..Default::default()
        });

        let ns = QualifiedName::new(vec!["com".into(), "example".into(), "hr".into()], None);
        assert_eq!(
            gener.namespace_to_iri(&ns),
            "http://example.org/com/example/hr"
        );
    }

    /// A package declared with an absolute IRI carries its terminator (`/` or
    /// `#`) in `parts[0]`. Sub-namespace path resolution must not double the
    /// slash or keep the hash: `namespace_iri` returns a bare stem, callers
    /// append the `#` fragment terminator themselves.
    #[test]
    fn test_namespace_iri_absolute_base_terminators() {
        let base = "http://example.org/"; // unused for absolute packages

        // Slash-terminated base: `<http://demo/test/>`.
        let root = QualifiedName::new(vec!["http://demo/test/".into()], None);
        let animals =
            QualifiedName::new(vec!["http://demo/test/".into(), "animals".into()], None);
        assert_eq!(namespace_iri(base, &root, None), "http://demo/test");
        assert_eq!(
            namespace_iri(base, &animals, None),
            "http://demo/test/animals"
        );

        // Hash-terminated base: `<http://demo/test#>`.
        let hroot = QualifiedName::new(vec!["http://demo/test#".into()], None);
        let hanimals =
            QualifiedName::new(vec!["http://demo/test#".into(), "animals".into()], None);
        assert_eq!(namespace_iri(base, &hroot, None), "http://demo/test");
        assert_eq!(
            namespace_iri(base, &hanimals, None),
            "http://demo/test/animals"
        );

        // The `#` the writers append then reconstitutes the correct prefixes:
        //   @prefix :        <http://demo/test#> .
        //   @prefix animals: <http://demo/test/animals#> .
        assert_eq!(
            format!("{}#", namespace_iri(base, &hroot, None)),
            "http://demo/test#"
        );
        assert_eq!(
            format!("{}#", namespace_iri(base, &hanimals, None)),
            "http://demo/test/animals#"
        );
    }

    #[test]
    fn test_namespace_to_iri_absolute_not_doubled() {
        // Package declared `package <http://example.com/TestFirst>:` → the IRI
        // is a single part already containing the full namespace. base_iri must
        // NOT be prepended, or we'd get `{base}{base}/...`.
        let gener = TurtleGenerator::new(TurtleOptions {
            base_iri: "http://example.com/TestFirst".to_string(),
            ..Default::default()
        });

        let pkg_ns = QualifiedName::new(vec!["http://example.com/TestFirst".into()], None);
        assert_eq!(
            gener.namespace_to_iri(&pkg_ns),
            "http://example.com/TestFirst"
        );

        // A file-level namespace (package IRI + file stem) keeps the stem.
        let file_ns =
            QualifiedName::new(vec!["http://example.com/TestFirst".into(), "first".into()], None);
        assert_eq!(
            gener.namespace_to_iri(&file_ns),
            "http://example.com/TestFirst/first"
        );
    }

    #[test]
    fn test_namespace_to_prefix() {
        let gener = TurtleGenerator::with_defaults();

        let base = QualifiedName::new(vec!["com".into(), "example".into()], None);
        let child = QualifiedName::new(
            vec![
                "com".into(),
                "example".into(),
                "hr".into(),
                "employee".into(),
            ],
            None,
        );

        assert_eq!(gener.namespace_to_prefix(&base, &base), "");
        assert_eq!(gener.namespace_to_prefix(&child, &base), "hr_employee");
    }

    #[test]
    fn test_escape_turtle_string() {
        assert_eq!(escape_turtle_string("hello"), "hello");
        assert_eq!(escape_turtle_string("hello\nworld"), "hello\\nworld");
        assert_eq!(escape_turtle_string("say \"hi\""), "say \\\"hi\\\"");
    }

    #[test]
    fn test_fact_literal_rendering() {
        use rowl::ast::Literal;
        let generator = TurtleGenerator::with_defaults();
        assert_eq!(
            generator.render_literal(&Literal::String { value: "Rex".to_string(), span: None }),
            "\"Rex\""
        );
        assert_eq!(
            generator.render_literal(&Literal::Int { value: 42, span: None }),
            "\"42\"^^xsd:integer"
        );
        assert_eq!(
            generator.render_literal(&Literal::Float { value: 3.14, span: None }),
            "\"3.14\"^^xsd:float"
        );
        assert_eq!(
            generator.render_literal(&Literal::Boolean { value: true, span: None }),
            "\"true\"^^xsd:boolean"
        );
    }

    #[test]
    fn test_temporal_literal_rendering() {
        use rowl::ast::{Literal, TemporalKind};
        let generator = TurtleGenerator::with_defaults();
        assert_eq!(
            generator.render_literal(&Literal::Temporal {
                kind: TemporalKind::Date,
                content: "June 1st 2026".to_string(),
                span: None,
            }),
            "\"2026-06-01\"^^xsd:date"
        );
        assert_eq!(
            generator.render_literal(&Literal::Temporal {
                kind: TemporalKind::Duration,
                content: "1y 6mo".to_string(),
                span: None,
            }),
            "\"P1Y6M\"^^xsd:duration"
        );
    }

    #[test]
    fn test_quantity_literal_rendering() {
        // Representation "L": value as written in the lexical slot, unit in a
        // canonical `unit:` datatype IRI. Unresolvable → plain-string fallback.
        use rowl::ast::Literal;
        let generator = TurtleGenerator::with_defaults();

        // Written value preserved; unit → canonical full-IRI datatype.
        assert_eq!(
            generator.render_literal(&Literal::Quantity {
                content: "9.81 m.s^(-2)".to_string(),
                span: None,
            }),
            "\"9.81\"^^<https://dolfin.dev/unit/m.s-2>"
        );
        assert_eq!(
            generator.render_literal(&Literal::Quantity {
                content: "42 km/h".to_string(),
                span: None,
            }),
            "\"42\"^^<https://dolfin.dev/unit/km.h-1>"
        );

        // A trailing `as` conversion is applied first, then rendered in L form.
        assert_eq!(
            generator.render_literal(&Literal::Quantity {
                content: "42 km/h as m/s".to_string(),
                span: None,
            }),
            "\"11.666666666666668\"^^<https://dolfin.dev/unit/m.s-1>"
        );

        // Dimensionless quantity → plain xsd:double (no unit datatype).
        assert_eq!(
            generator.render_literal(&Literal::Quantity {
                content: "42".to_string(),
                span: None,
            }),
            "\"42\"^^xsd:double"
        );

        // Unknown unit → plain-string fallback (S007 reports the real error).
        assert_eq!(
            generator.render_literal(&Literal::Quantity {
                content: "42 zonks".to_string(),
                span: None,
            }),
            "\"42 zonks\""
        );
    }

    #[test]
    fn test_temporal_unresolved_falls_back_to_string() {
        // INTERIM behaviour, not desired semantics: temporal literals that fail
        // to resolve currently render as a plain string so generation never
        // crashes. Two cases fall here today, and BOTH should eventually be
        // surfaced as author diagnostics instead:
        //   1. numeric date without an inline mask (needs file-level @locale,
        //      which is not yet plumbed into the TemporalContext);
        //   2. a kind mismatch, e.g. date(7d) — the value parses as a duration
        //      but was declared date(). This is a real error the "typed
        //      constructor" design promised to reject; the plain-string output
        //      below is a placeholder until diagnostics are threaded through
        //      render_literal (bundled with the @locale work).
        use rowl::ast::{Literal, TemporalKind};
        let generator = TurtleGenerator::with_defaults();
        assert_eq!(
            generator.render_literal(&Literal::Temporal {
                kind: TemporalKind::Date,
                content: "01/06/2026".to_string(),
                span: None,
            }),
            "\"01/06/2026\""
        );
        assert_eq!(
            generator.render_literal(&Literal::Temporal {
                kind: TemporalKind::Date,
                content: "7d".to_string(),
                span: None,
            }),
            "\"7d\""
        );
    }

    #[test]
    fn test_fact_reference_formatting() {
        use rowl::ast::FactValue;
        // Verify the pattern matches produce the expected Turtle terms
        let local_ref = FactValue::Reference { qualifier: None, name: "John".to_string(), span: None };
        let turtle_term = match &local_ref {
            FactValue::Reference { qualifier: None, name, .. } => format!(":{}", name),
            _ => panic!(),
        };
        assert_eq!(turtle_term, ":John");

        let prefixed_ref = FactValue::Reference { qualifier: Some("ex".to_string()), name: "Mary".to_string(), span: None };
        let turtle_term2 = match &prefixed_ref {
            FactValue::Reference { qualifier: Some(q), name, .. } => format!("{}:{}", q, name),
            _ => panic!(),
        };
        assert_eq!(turtle_term2, "ex:Mary");
    }

    fn int_expr(n: i64) -> Expr {
        Expr::Literal {
            value: Literal::Int { value: n, span: None },
            span: None,
        }
    }

    fn bin(op: BinaryOp, left: Expr, right: Expr) -> Expr {
        Expr::BinaryOp {
            op,
            left: Box::new(left),
            right: Box::new(right),
            span: None,
        }
    }

    #[test]
    fn test_add_chain_flattened_to_single_sum() {
        // `1 + 2 + 3` parses as `(1 + 2) + 3` → one N-ary math:sum triple.
        let expr = bin(BinaryOp::Add, bin(BinaryOp::Add, int_expr(1), int_expr(2)), int_expr(3));
        let mut generator = TurtleGenerator::with_defaults();
        let mut out = String::new();
        let result = generator.render_expr(&mut out, &expr, false);
        assert_eq!(out.matches("math:sum").count(), 1);
        let one = generator.render_literal(&Literal::Int { value: 1, span: None });
        let two = generator.render_literal(&Literal::Int { value: 2, span: None });
        let three = generator.render_literal(&Literal::Int { value: 3, span: None });
        assert!(
            out.contains(&format!("( {} {} {} ) math:sum {}", one, two, three, result)),
            "got: {}",
            out
        );
    }

    #[test]
    fn test_variable_operand_renders_as_var_term() {
        // `?x + 1` → the variable flows through as its own Turtle term (`?x`),
        // paired with the literal in a `math:sum` triple.
        let var = Expr::Variable { name: "?x".to_string(), span: None };
        let expr = bin(BinaryOp::Add, var, int_expr(1));
        let mut generator = TurtleGenerator::with_defaults();
        let mut out = String::new();
        let result = generator.render_expr(&mut out, &expr, false);
        let one = generator.render_literal(&Literal::Int { value: 1, span: None });
        assert!(
            out.contains(&format!("( ?x {} ) math:sum {}", one, result)),
            "got: {}",
            out
        );
    }

    #[test]
    fn test_mul_chain_flattened_to_single_product() {
        let expr = bin(BinaryOp::Mul, bin(BinaryOp::Mul, int_expr(2), int_expr(3)), int_expr(4));
        let mut generator = TurtleGenerator::with_defaults();
        let mut out = String::new();
        let result = generator.render_expr(&mut out, &expr, false);
        assert_eq!(out.matches("math:product").count(), 1);
        let two = generator.render_literal(&Literal::Int { value: 2, span: None });
        let three = generator.render_literal(&Literal::Int { value: 3, span: None });
        let four = generator.render_literal(&Literal::Int { value: 4, span: None });
        assert!(
            out.contains(&format!("( {} {} {} ) math:product {}", two, three, four, result)),
            "got: {}",
            out
        );
    }

    #[test]
    fn test_sub_chain_stays_pairwise() {
        // `1 - 2 - 3` = `(1 - 2) - 3`. Non-associative → two binary differences.
        let expr = bin(BinaryOp::Sub, bin(BinaryOp::Sub, int_expr(1), int_expr(2)), int_expr(3));
        let mut generator = TurtleGenerator::with_defaults();
        let mut out = String::new();
        generator.render_expr(&mut out, &expr, false);
        assert_eq!(out.matches("math:difference").count(), 2, "got: {}", out);
    }

    #[test]
    fn test_mixed_op_not_over_flattened() {
        // `1 * 2 + 3` = `(1 * 2) + 3`. Top sum has 2 operands; product renders separately.
        let expr = bin(BinaryOp::Add, bin(BinaryOp::Mul, int_expr(1), int_expr(2)), int_expr(3));
        let mut generator = TurtleGenerator::with_defaults();
        let mut out = String::new();
        generator.render_expr(&mut out, &expr, false);
        assert_eq!(out.matches("math:product").count(), 1, "got: {}", out);
        assert_eq!(out.matches("math:sum").count(), 1, "got: {}", out);
    }
}
