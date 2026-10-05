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
    // owner John — a fact of this file, in this file's namespace
    assert!(
        ttl.contains("animals:rex animals:owner animals:John ."),
        "expected reference animals:John:\n{ttl}"
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
    // `is spouse of jane` on John → jane spouse John
    // the value (jane) becomes the subject, spouse the predicate, John the object
    assert!(
        ttl.contains("animals:jane animals:spouse animals:John ."),
        "expected inverse triple jane spouse John:\n{ttl}"
    );
}

#[test]
fn test_fact_bare_property_resolved_by_type() {
    let files: std::collections::HashMap<String, String> = [
        ("package.dlf", "package <http://example.org/zoo>:\n  dolfin_version \"1\"\n  version \"0.1.0\"\n  author \"test\"\n"),
        ("person.dlf", "concept Person:\n  has name: string\n"),
        ("animal.dlf", "concept Animal:\n  has name: string\n"),
        ("example.dlf", "fact jack a person.Person\n  name \"Jack\"\n  pet [\n    a animal.Animal\n    name \"Tom\"\n  ]\n\nfact rex a animal.Animal\n  name \"Rex\"\n"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect();
    let package = rowl::package::load_package_from_memory(&files).expect("package should load");
    let ttl = TurtleGenerator::new(TurtleOptions {
        base_iri: "http://example.org/".to_string(),
        include_comments: false,
        include_rules_as_comments: false,
        include_queries_as_comments: false,
    })
    .generate(&package)
    .expect("generation should succeed");

    assert!(ttl.contains("example:jack person:name \"Jack\""), "{ttl}");
    assert!(ttl.contains("example:rex animal:name \"Rex\""), "{ttl}");
    assert!(ttl.contains(" animal:name \"Tom\""), "{ttl}");
    assert!(!ttl.contains("example:name"), "{ttl}");
}

#[test]
fn test_rule_bare_property_resolved_by_type() {
    let files: std::collections::HashMap<String, String> = [
        ("package.dlf", "package <http://example.org/zoo>:\n  dolfin_version \"1\"\n  version \"0.1.0\"\n  author \"test\"\n"),
        ("person.dlf", "concept Person:\n  has name: string\n  has pet: animal.Animal\n  has nick: string\n"),
        ("animal.dlf", "concept Animal:\n  has name: string\n"),
        (
            "example.dlf",
            "rule nick_from_pet:\n  match:\n    ?x a person.Person\n    ?x pet [ name ?n ]\n  then:\n    ?x nick ?n\n",
        ),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect();
    let package = rowl::package::load_package_from_memory(&files).expect("package should load");
    let n3 = irukame::rules_as_n3(&package).expect("n3 generates");

    assert!(n3.contains(" animal:name ?n"), "{n3}");
    assert!(n3.contains("?x person:nick ?n"), "{n3}");
    assert!(!n3.contains("example:name"), "{n3}");
}

#[test]
fn test_query_bare_property_resolved_by_type() {
    let files: std::collections::HashMap<String, String> = [
        ("package.dlf", "package <http://example.org/zoo>:\n  dolfin_version \"1\"\n  version \"0.1.0\"\n  author \"test\"\n"),
        ("person.dlf", "concept Person:\n  has name: string\n  has pet: animal.Animal\n\nquery people:\n  ?p a Person\n"),
        ("animal.dlf", "concept Animal:\n  has name: string\n"),
        ("example.dlf", "query pets:\n  ?x a person.Person\n    name ?n\n    pet [ name ?m ]\n"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect();
    let package = rowl::package::load_package_from_memory(&files).expect("package should load");
    let ttl = TurtleGenerator::new(TurtleOptions {
        base_iri: "http://example.org/".to_string(),
        include_comments: false,
        include_rules_as_comments: false,
        include_queries_as_comments: true,
    })
    .generate(&package)
    .expect("generation should succeed");

    assert!(ttl.contains("?x person:name ?n"), "{ttl}");
    assert!(ttl.contains("?x person:pet ?_v"), "{ttl}");
    assert!(ttl.contains(" animal:name ?m"), "{ttl}");
    assert!(ttl.contains("?x a person:Person ."), "{ttl}");
    assert!(ttl.contains("?p a person:Person ."), "{ttl}");
}
