//! Tests for @iri_name annotation in Turtle generation.
//!
//! @iri_name on a file overrides the last segment of that file's namespace IRI.
//! Without it, the IRI is derived from package name + path + filename.

use irukame::{TurtleGenerator, TurtleOptions};
use rowl::package::load_package_from_memory;
use std::collections::HashMap;

fn base_opts() -> TurtleOptions {
    TurtleOptions {
        base_iri: "http://example.org/".to_string(),
        include_comments: false,
        include_rules_as_comments: false,
        include_queries_as_comments: false,
    }
}

fn generate(files: HashMap<&str, &str>) -> String {
    let owned: HashMap<String, String> = files
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let package = load_package_from_memory(&owned).expect("package load failed");
    let mut generator = TurtleGenerator::new(base_opts());
    generator.generate(&package).expect("generation failed")
}

const MANIFEST: &str = r#"package animals:
  dolfin_version "1"
  version "0.1.0"
  author "test"
"#;

#[test]
fn test_no_iri_name_uses_namespace_path() {
    let ttl = generate(HashMap::from([
        ("package.dlf", MANIFEST),
        ("mammals.dlf", "concept Mammal\n"),
    ]));
    assert!(
        ttl.contains("<http://example.org/animals/mammals#>"),
        "expected namespace-derived prefix IRI:\n{ttl}"
    );
}

#[test]
fn test_iri_name_overrides_prefix_iri() {
    let ttl = generate(HashMap::from([
        ("package.dlf", MANIFEST),
        (
            "mammals.dlf",
            "@iri_name <http://animals.kingdom/Mammalian>\n\nconcept Mammal\n",
        ),
    ]));
    assert!(
        ttl.contains("<http://animals.kingdom/Mammalian#>"),
        "expected iri_name value in prefix IRI:\n{ttl}"
    );
    assert!(
        !ttl.contains("<http://example.org/animals/mammals#>"),
        "raw namespace IRI must not appear when iri_name is set:\n{ttl}"
    );
}

#[test]
fn test_iri_name_overrides_part_of_prefix_iri() {
    let ttl = generate(HashMap::from([
        ("package.dlf", MANIFEST),
        ("mammals.dlf", "@iri_name \"Mammalian\"\n\nconcept Mammal\n"),
    ]));
    assert!(
        ttl.contains("<http://example.org/animals/Mammalian#>"),
        "expected iri_name value in prefix IRI:\n{ttl}"
    );
    assert!(
        !ttl.contains("<http://example.org/animals/mammals#>"),
        "raw namespace IRI must not appear when iri_name is set:\n{ttl}"
    );
}

#[test]
fn test_iri_name_file_is_defined_by_chain() {
    let ttl = generate(HashMap::from([
        ("package.dlf", MANIFEST),
        (
            "mammals.dlf",
            "@iri_name <http://animals.kingdom/Mammalian>\n\nconcept Mammal\n",
        ),
    ]));
    // Entities point isDefinedBy at the override file resource, not the
    // canonical path-derived one.
    assert!(
        ttl.contains("rdfs:isDefinedBy <http://animals.kingdom/Mammalian>"),
        "expected entity isDefinedBy to use @iri_name override:\n{ttl}"
    );
    // The override file resource chains back to the canonical file resource
    // so mekarui can still recover mammals.dlf.
    assert!(
        ttl.contains(
            "<http://animals.kingdom/Mammalian> rdfs:isDefinedBy <http://example.org/animals/mammals>"
        ),
        "expected override file resource to chain back to canonical:\n{ttl}"
    );
}

#[test]
fn test_iri_name_concept_uses_prefix_label() {
    let ttl = generate(HashMap::from([
        ("package.dlf", MANIFEST),
        (
            "mammals.dlf",
            "@iri_name <http://animals.kingdom/Mammalian>\n\nconcept Mammal\n",
        ),
    ]));
    // The concept reference still uses the prefix label derived from the filename
    assert!(
        ttl.contains("mammals:Mammal"),
        "concept must use prefix label (not iri_name):\n{ttl}"
    );
}

#[test]
fn test_iri_name_does_not_affect_sibling_file() {
    let ttl = generate(HashMap::from([
        ("package.dlf", MANIFEST),
        (
            "mammals.dlf",
            "@iri_name <http://animals.kingdom/Mammalian>\n\nconcept Mammal\n",
        ),
        ("reptiles.dlf", "concept Reptile\n"),
    ]));
    // Sibling file without iri_name keeps normal IRI
    assert!(
        ttl.contains("<http://example.org/animals/reptiles#>"),
        "sibling file must keep namespace-derived IRI:\n{ttl}"
    );
    // And iri_name file still overrides
    assert!(
        ttl.contains("<http://animals.kingdom/Mammalian#>"),
        "iri_name file must still override:\n{ttl}"
    );
}

#[test]
fn test_iri_name_does_not_affect_sibling_concepts() {
    let ttl = generate(HashMap::from([
        ("package.dlf", MANIFEST),
        (
            "anymal.dlf",
            "concept Mammal:\n  @iri_name <http://animals.kingdom/Mammals>\n\nconcept Reptile\n",
        ),
    ]));
    // Sibling file without iri_name keeps normal IRI
    assert!(
        ttl.contains("<http://example.org/animals/anymal#Reptile>"),
        "sibling file must keep namespace-derived IRI:\n{ttl}"
    );
    // And iri_name file still overrides
    assert!(
        ttl.contains("<http://animals.kingdom/Mammals>"),
        "iri_name file must still override:\n{ttl}"
    );
}

#[test]
fn test_iri_name_overrides_dimension_range_iri() {
    let ttl = generate(HashMap::from([
        ("package.dlf", MANIFEST),
        (
            "units.dlf",
            "concept Mass:\n  @iri_name <http://qudt.org/vocab/quantitykind/Mass>\n",
        ),
        (
            "animal.dlf",
            "concept Animal:\n  has weight: unit.Mass\n",
        ),
    ]));
    assert!(
        ttl.contains("<http://qudt.org/vocab/quantitykind/Mass>"),
        "expected QUDT quantity-kind IRI as rdfs:range:\n{ttl}"
    );
    assert!(
        !ttl.contains("dolfin.dev/dimension/"),
        "minted dimension IRI must not appear when @iri_name override is present:\n{ttl}"
    );
}

#[test]
fn test_dimension_range_iri_uses_builtin_qudt_without_units_dlf() {
    // No units.dlf (or any `concept Mass`) in the project at all — the
    // built-in QUDT quantity-kind mapping in dolfin-units still applies.
    let ttl = generate(HashMap::from([
        ("package.dlf", MANIFEST),
        (
            "animal.dlf",
            "concept Animal:\n  has weight: unit.Mass\n",
        ),
    ]));
    assert!(
        ttl.contains("<http://qudt.org/vocab/quantitykind/Mass>"),
        "expected built-in QUDT quantity-kind IRI with no units.dlf present:\n{ttl}"
    );
    assert!(
        !ttl.contains("dolfin.dev/dimension/"),
        "minted dimension IRI must not appear when the dimension has a QUDT mapping:\n{ttl}"
    );
}

/// A concept-level `@iri_name` in a file that also has a file-level `@iri_name` must not move
/// the file's other concepts: they are declared in the file-level namespace, where every
/// reference (`prefix:Name`) points. (Used to declare them at the canonical path-derived IRI.)
#[test]
fn test_concept_iri_name_keeps_siblings_in_file_level_namespace() {
    let ttl = generate(HashMap::from([
        ("package.dlf", MANIFEST),
        (
            "mammals.dlf",
            "@iri_name <http://animals.kingdom/Mammalian>\n\nconcept Ext:\n  @iri_name <http://other.org/Ext>\n\nconcept Mammal\n",
        ),
    ]));
    assert!(ttl.contains("<http://animals.kingdom/Mammalian#Mammal> rdf:type"), "{ttl}");
    assert!(!ttl.contains("animals/mammals#Mammal"), "canonical IRI leaked:\n{ttl}");
    assert!(ttl.contains("<http://other.org/Ext> rdf:type"), "{ttl}");
}

// ---- namespace terminator: a declared `#` / `/` is kept, only a bare IRI gets the default `#` ----

/// File-level `@iri_name <http://ex.org/ns/>`: the namespace keeps its `/`, so entities are
/// `http://ex.org/ns/Foo`, not `http://ex.org/ns/#Foo`.
#[test]
fn test_slash_terminated_file_level_iri_name() {
    let ttl = generate(HashMap::from([
        ("package.dlf", MANIFEST),
        (
            "ns.dlf",
            "@iri_name <http://ex.org/ns/>\n\nconcept Foo:\n  has name: string\n\nconcept Bar:\n  sub Foo\n  has boss: one Foo\n",
        ),
    ]));
    assert!(ttl.contains("@prefix ns: <http://ex.org/ns/> ."), "{ttl}");
    assert!(!ttl.contains("http://ex.org/ns/#") && !ttl.contains("http://ex.org/ns##"), "{ttl}");
    // references and declarations share one namespace; `isDefinedBy` is the bare file resource
    assert!(ttl.contains("ns:Foo rdf:type rdfs:Class"), "{ttl}");
    assert!(ttl.contains("rdfs:subClassOf ns:Foo"), "{ttl}");
    assert!(ttl.contains("rdfs:isDefinedBy <http://ex.org/ns/>"), "{ttl}");
}

/// A declared `#` is kept too (it used to double up to `ns##`); the bare form still gets `#`.
#[test]
fn test_hash_terminated_and_bare_iri_name_are_the_same_namespace() {
    let mk = |iri: &str| {
        let dlf = format!("@iri_name <{iri}>\n\nconcept Foo\n");
        generate(HashMap::from([("package.dlf", MANIFEST), ("ns.dlf", dlf.as_str())]))
    };
    let (bare, hash) = (mk("http://ex.org/ns"), mk("http://ex.org/ns#"));
    assert!(bare.contains("@prefix ns: <http://ex.org/ns#> ."), "{bare}");
    assert!(hash.contains("@prefix ns: <http://ex.org/ns#> ."), "{hash}");
    assert!(!hash.contains("##"), "{hash}");
}

/// A concept-level `@iri_name` in a `/`-terminated file-level namespace: the sibling is declared
/// in that same namespace (`ns/Mammal`, matching every `ns:Mammal` reference), the overridden
/// concept sits on its own exact IRI.
#[test]
fn test_slash_terminated_file_level_with_concept_level_iri_name() {
    let ttl = generate(HashMap::from([
        ("package.dlf", MANIFEST),
        (
            "mammals.dlf",
            "@iri_name <http://animals.kingdom/m/>\n\nconcept Ext:\n  @iri_name <http://other.org/Ext>\n\nconcept Mammal:\n  has ext: one Ext\n",
        ),
    ]));
    assert!(ttl.contains("<http://animals.kingdom/m/Mammal> rdf:type"), "{ttl}");
    assert!(ttl.contains("<http://other.org/Ext> rdf:type"), "{ttl}");
    assert!(!ttl.contains("m/#") && !ttl.contains("animals/mammals#Mammal"), "{ttl}");
    assert!(ttl.contains("@prefix mammals: <http://animals.kingdom/m/> ."), "{ttl}");
}

/// A `/`-terminated `@iri_name` and the package's own base coexist: the owl:Ontology IRI stays
/// the bare package IRI (ontology IRIs are not entity namespaces), the `:` prefix is unchanged.
#[test]
fn test_slash_terminated_iri_name_leaves_package_base_alone() {
    let ttl = generate(HashMap::from([
        ("package.dlf", "package <http://ex.org/ns/>:\n  dolfin_version \"1\"\n  version \"0.1.0\"\n"),
        ("ns.dlf", "@iri_name <http://ex.org/ns/>\n\nconcept Foo\n"),
    ]));
    assert!(ttl.contains("@prefix : <http://ex.org/ns#> ."), "{ttl}");
    assert!(ttl.contains("<http://ex.org/ns> rdf:type owl:Ontology"), "{ttl}");
    assert!(ttl.contains("ns:Foo rdf:type rdfs:Class"), "{ttl}");
    assert!(ttl.contains("@prefix ns: <http://ex.org/ns/> ."), "{ttl}");
}

#[test]
fn test_namespace_prefix_iri_terminators() {
    use irukame::namespace_prefix_iri;
    use rowl::ast::{IriNameValue, QualifiedName};
    let ns = QualifiedName::new(vec!["animals".into(), "mammals".into()], None);
    let abs = |s: &str| Some(IriNameValue::AbsoluteUri(s.to_string()));
    let f = |o: Option<IriNameValue>| namespace_prefix_iri("http://example.org/", &ns, o.as_ref());
    assert_eq!(f(None), "http://example.org/animals/mammals#");
    assert_eq!(f(abs("http://ex.org/ns")), "http://ex.org/ns#");
    assert_eq!(f(abs("http://ex.org/ns#")), "http://ex.org/ns#");
    assert_eq!(f(abs("http://ex.org/ns/")), "http://ex.org/ns/");
    assert_eq!(f(Some(IriNameValue::LocalSegment("Mam".into()))), "http://example.org/animals/Mam#");
}
