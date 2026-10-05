# irukame

Codegen crate for dolfin. Takes a loaded `rowl::package::Package` (the dolfin AST) and emits Turtle/OWL and N3 rules.

## Modules

- **turtle** — RDF/OWL Turtle and N3 rules (the bulk of the crate: SKOS, facts, axioms, temporal, URI/IRI handling, `#@ glossary:` definitions)
- **plugin** — `DolfinPlugin` trait: a generic hook for `#@`-annotation-driven generators (SHACL, SparNatural config, etc.), so new output targets plug in without touching the core parser
- **decl_key** — `DeclKey` / `DeclKind`: declaration identity shared with `mekarui` (Turtle import), so both sides key the same declaration alike

## Dependencies

`rowl`, `dolfin-datetime`, `dolfin-units`, `dolfin-analysis`, `dolfin-query`, `thiserror`, `serde`.

## Usage

The top-level helper `rules_as_n3()` is a shortcut for `TurtleGenerator::with_defaults().generate_n3_rules()`; its output is N3 ready for a rule engine.

```rust
let n3 = irukame::rules_as_n3(&package)?;
```

## Tests

Turtle (snapshots, SKOS, facts, temporal, axioms, URI/IRI, enums, cross-file references, declaration ranges, rule quantifiers, equality), N3 rule output, user-defined inverses.
