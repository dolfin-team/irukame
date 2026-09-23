# irukame

Codegen crate for dolfin. Takes a loaded `rowl::package::Package` (dolfin AST) and emits output formats.

## Modules

- **turtle** — RDF/OWL Turtle + N3 rules (biggest module: SKOS, facts, axioms, temporal, URI/IRI handling)
- **glossary** — human-readable glossary docs (concepts/properties/fields) from AST
- **sparnatural** — SparNatural search-widget config, driven by `#@sparnatural` annotations
- **plugin** — `DolfinPlugin` trait: generic hook for `#@`-annotation-driven generators (SHACL, OWL config, etc.), so new output targets plug in without touching core parser

## Dependencies

`dolfin-datetime`, `dolfin-units`, `dolfin-analysis`, `glossary-ir`, `dolfin-query`, `oxigraph`.

## Usage

Top-level helper `rules_as_n3()` — shortcut to `TurtleGenerator::with_defaults().generate_n3_rules()`, feeds `retox::load_n3_rules_from_str`.

```rust
let n3 = irukame::rules_as_n3(&package)?;
```

## Tests

Turtle (snapshots, SKOS, facts, temporal, axioms, URI/IRI), glossary, sparnatural, user-inverse.
