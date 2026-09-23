//! Tests for PropertyAxiom emission in the Turtle generator.

use irukame::{TurtleGenerator, TurtleOptions};
use rowl::package::load_package;
use std::path::PathBuf;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/axioms")
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
fn test_axiom_symmetric_reflexive() {
    let ttl = run_generator();
    assert!(
        ttl.contains("owl:SymmetricProperty"),
        "expected owl:SymmetricProperty:\n{ttl}"
    );
    assert!(
        ttl.contains("owl:ReflexiveProperty"),
        "expected owl:ReflexiveProperty:\n{ttl}"
    );
}

#[test]
fn test_axiom_sub_property() {
    let ttl = run_generator();
    assert!(
        ttl.contains("rdfs:subPropertyOf family:friend_of"),
        "expected rdfs:subPropertyOf family:friend_of:\n{ttl}"
    );
}

#[test]
fn test_axiom_inverse_of_in_has() {
    let ttl = run_generator();
    assert!(
        ttl.contains("owl:inverseOf family:organizes"),
        "expected owl:inverseOf family:organizes:\n{ttl}"
    );
}

#[test]
fn test_axiom_equivalent_to_chain() {
    let ttl = run_generator();
    assert!(
        ttl.contains("owl:propertyChainAxiom ( family:parent family:parent )"),
        "expected chain axiom for grand_parent:\n{ttl}"
    );
}

#[test]
fn test_axiom_equivalent_to_inverse_in_chain() {
    let ttl = run_generator();
    assert!(
        ttl.contains("owl:propertyChainAxiom ( family:parent [ owl:inverseOf family:parent ] )"),
        "expected inverse-in-chain for sibling:\n{ttl}"
    );
}

#[test]
fn test_axiom_equivalent_to_one_or_more() {
    let ttl = run_generator();
    assert!(
        ttl.contains("owl:TransitiveProperty"),
        "expected owl:TransitiveProperty for ancestor:\n{ttl}"
    );
    assert!(
        ttl.contains("owl:propertyChainAxiom ( family:parent family:ancestor )"),
        "expected chain axiom for ancestor+:\n{ttl}"
    );
}
