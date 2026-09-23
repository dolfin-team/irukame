//! Tests for URI-literal prefix declarations and `alias:Local` prefixed-name references.

use irukame::{TurtleGenerator, TurtleOptions};
use rowl::package::{load_package, load_package_from_memory};
use std::collections::HashMap;
use std::path::PathBuf;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/uri_prefix")
}

fn run_generator() -> String {
    let package = load_package(fixture_path()).expect("fixture package should load");
    let mut generator = TurtleGenerator::new(TurtleOptions {
        base_iri: "http://example.org/".to_string(),
        include_comments: false,
        include_rules_as_comments: false,
        include_queries_as_comments: false,
    });
    generator
        .generate(&package)
        .expect("generation should succeed")
}

#[test]
fn test_uri_prefix_declaration_emitted() {
    let ttl = run_generator();
    assert!(
        ttl.contains("@prefix zoo: <http://example.com/animals/> ."),
        "expected URI prefix declaration:\n{ttl}"
    );
}

#[test]
fn test_prefixed_name_reference_resolved() {
    let ttl = run_generator();
    assert!(
        ttl.contains("rdfs:subClassOf zoo:Animal"),
        "expected zoo:Animal as sub reference:\n{ttl}"
    );
}

fn make_options() -> TurtleOptions {
    TurtleOptions {
        base_iri: "http://example.org/".to_string(),
        include_comments: false,
        include_rules_as_comments: false,
        include_queries_as_comments: false,
    }
}

/// End-to-end: `prefix <http://example.com/onto/> as ee` + `concept Machin: sub ee:Truc`
/// emits `@prefix ee: <http://example.com/onto/> .` and `rdfs:subClassOf ee:Truc`.
#[test]
fn test_uri_prefix_sub_machin_truc() {
    let mut files = HashMap::new();
    files.insert(
        "package.dlf".to_string(),
        "package <http://example.org/test>:\n  dolfin_version \"1\"\n  version \"0.1.0\"\n  author \"test\"\n".to_string(),
    );
    files.insert(
        "machin.dlf".to_string(),
        "prefix <http://example.com/onto/> as ee\n\nconcept Machin:\n  sub ee:Truc\n".to_string(),
    );

    let package = load_package_from_memory(&files).expect("package should load");
    let ttl = TurtleGenerator::new(make_options())
        .generate(&package)
        .expect("generation should succeed");

    assert!(
        ttl.contains("@prefix ee: <http://example.com/onto/> ."),
        "expected URI prefix declaration:\n{ttl}"
    );
    assert!(
        ttl.contains("rdfs:subClassOf ee:Truc"),
        "expected rdfs:subClassOf ee:Truc:\n{ttl}"
    );
}
