use irukame::{TurtleGenerator, TurtleOptions};
use rowl::package::load_package;
use std::path::PathBuf;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/facts")
}

fn run_generator() -> String {
    let package = load_package(fixture_path()).expect("fixture package should load");
    let mut generator = TurtleGenerator::new(TurtleOptions {
        base_iri: "http://example.org/".to_string(),
        include_comments: false,
        include_rules_as_comments: false,
        include_queries_as_comments: false,
    });
    generator.generate(&package).expect("generation should succeed")
}

fn run_schema_only() -> String {
    let package = load_package(fixture_path()).expect("fixture package should load");
    let mut generator = TurtleGenerator::new(TurtleOptions {
        base_iri: "http://example.org/".to_string(),
        include_comments: false,
        include_rules_as_comments: false,
        include_queries_as_comments: false,
    });
    generator.generate_schema_only(&package).expect("generation should succeed")
}

#[test]
fn test_schema_only_omits_facts() {
    let schema = run_schema_only();
    assert!(
        !schema.contains("owl:NamedIndividual"),
        "schema-only output must not contain fact individuals:\n{schema}"
    );
    // T-box still present.
    assert!(
        schema.contains("rdfs:Class"),
        "schema-only output should still contain concept classes:\n{schema}"
    );
    // The full output (with facts) must be a superset.
    let full = run_generator();
    assert!(
        full.contains("owl:NamedIndividual"),
        "full output must contain fact individuals:\n{full}"
    );
}

#[test]
fn test_fact_named_individual() {
    let ttl = run_generator();
    assert!(
        ttl.contains("owl:NamedIndividual"),
        "expected owl:NamedIndividual in output:\n{ttl}"
    );
    let count = ttl.matches("owl:NamedIndividual").count();
    assert_eq!(count, 4, "expected 4 facts, each with NamedIndividual, got {count}:\n{ttl}");
}

#[test]
fn test_fact_multi_type() {
    let ttl = run_generator();
    // felix is a Dog and NeverVaccinated — both types should appear in rdf:type list
    assert!(
        ttl.contains("NeverVaccinated"),
        "expected NeverVaccinated type for felix:\n{ttl}"
    );
    assert!(ttl.contains("Dog"), "expected Dog type:\n{ttl}");
}

#[test]
fn test_fact_property_literal() {
    let ttl = run_generator();
    assert!(
        ttl.contains("\"Rex\""),
        "expected string literal 'Rex':\n{ttl}"
    );
    assert!(
        ttl.contains("\"3\"^^xsd:integer"),
        "expected integer literal 3:\n{ttl}"
    );
    assert!(
        ttl.contains("^^xsd:float"),
        "expected float literal:\n{ttl}"
    );
    assert!(
        ttl.contains("\"true\"^^xsd:boolean"),
        "expected boolean literal true:\n{ttl}"
    );
}

#[test]
fn test_fact_reference() {
    let ttl = run_generator();
    // owner :John — fact reference serialized with colon prefix
    assert!(
        ttl.contains(":John"),
        "expected reference :John:\n{ttl}"
    );
}

#[test]
fn test_fact_anonymous_block() {
    let ttl = run_generator();
    // blank node emitted for vaccination block
    assert!(
        ttl.contains("_:"),
        "expected blank node in output:\n{ttl}"
    );
    // sub-triple with "Rabies"
    assert!(
        ttl.contains("\"Rabies\""),
        "expected blank node sub-triple 'Rabies':\n{ttl}"
    );
}

#[test]
fn test_fact_type_hint_block() {
    let ttl = run_generator();
    // blank node gets rdf:type :VaccinationRecord from type hint `a VaccinationRecord`
    assert!(
        ttl.contains("VaccinationRecord"),
        "expected VaccinationRecord type hint on blank node:\n{ttl}"
    );
    // the blank node should have an rdf:type line
    let has_blank_type = ttl.contains("_:") && ttl.contains("rdf:type");
    assert!(has_blank_type, "expected blank node with rdf:type:\n{ttl}");
}

#[test]
fn test_fact_inverse() {
    let ttl = run_generator();
    // `is spouse of :jane` on John → :jane <spouse_prop> <John_ref>
    // the value (:jane) becomes the subject, spouse the predicate, John the object
    assert!(
        ttl.contains(":jane"),
        "expected :jane in inverse triple:\n{ttl}"
    );
    assert!(
        ttl.contains("spouse"),
        "expected spouse property in inverse triple:\n{ttl}"
    );
}
