// Verifies the user's `is {prop} of` inverse-usage examples translate correctly
// across all three contexts: fact (Turtle), rule (N3), query (SPARQL).

use dolfin_query::to_sparql;
use irukame::{rules_as_n3, TurtleGenerator, TurtleOptions};
use rowl::package::load_package;
use rowl::parser::parse_ontology;
use std::path::PathBuf;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/inverse_usage")
}

fn opts() -> TurtleOptions {
    TurtleOptions {
        base_iri: "http://example.org/".to_string(),
        include_comments: false,
        include_rules_as_comments: false,
        include_queries_as_comments: false,
    }
}

#[test]
fn fact_inverse_reverses_triple() {
    let pkg = load_package(fixture()).expect("package loads");
    let ttl = TurtleGenerator::new(opts())
        .generate(&pkg)
        .expect("turtle generates");
    eprintln!("=== TURTLE ===\n{ttl}");
    // `Znd … is actor of Spiderman` → `:Spiderman :actor :Znd .`
    assert!(ttl.contains(":Spiderman"), "expected Spiderman subject:\n{ttl}");
    assert!(ttl.contains("actor"), "expected actor predicate:\n{ttl}");
    assert!(ttl.contains(":Znd"), "expected Znd object:\n{ttl}");
}

#[test]
fn rule_inverse_match_to_n3() {
    let pkg = load_package(fixture()).expect("package loads");
    let n3 = rules_as_n3(&pkg).expect("n3 generates");
    eprintln!("=== N3 RULE ===\n{n3}");
    // `?one is actor of [ a Movie, actor ?two ]` desugars to an inverse triple:
    // `?n actor ?one . ?n a :Movie . ?n actor ?two .`  =>  `?one played_with ?two`.
    assert!(n3.contains("=>"), "expected an N3 implication:\n{n3}");
    assert!(n3.contains(":Movie"), "expected the Movie type from the block:\n{n3}");
    assert!(n3.contains("played_with"), "expected the consequent property:\n{n3}");
    // The antecedent must contain `actor` twice (the inverse link + the block's actor ?two).
    let actor_count = n3.matches(":actor").count();
    assert!(actor_count >= 2, "expected >=2 actor triples, got {actor_count}:\n{n3}");
}

#[test]
fn query_inverse_constraint_to_sparql() {
    let src = std::fs::read_to_string(fixture().join("cinema.dlf")).unwrap();
    let result = parse_ontology(&src);
    assert!(!result.has_errors(), "parse errors: {:?}", result.errors());
    let onto = result.ontology.unwrap();
    let q = onto.queries().into_iter().next().expect("a query").clone();
    let sparql = to_sparql(&q, &[&q]).expect("sparql generates");
    eprintln!("=== SPARQL ===\n{sparql}");
    // `?movie1 director [ is director of ?movie2 ]`:
    //   ?movie1 director ?v .
    //   ?movie2 director ?v .     (inverse: ?movie2 directs the same ?v)
    assert!(sparql.contains("SELECT"), "expected SELECT:\n{sparql}");
    assert!(sparql.contains("director"), "expected director predicate:\n{sparql}");
    assert!(sparql.contains("?movie2"), "expected the inverse subject ?movie2:\n{sparql}");
    let director_count = sparql.matches("director").count();
    assert!(director_count >= 2, "expected >=2 director triples, got {director_count}:\n{sparql}");
}

// The user's literal query needs `?movie2 != ?movie1` (var-to-var) — unsupported
// *inside* a constraint block (Expr has no variable variant). Hoisting it to a
// top-level body filter is the end-to-end equivalent and works today.
#[test]
fn query_inverse_with_hoisted_self_exclusion() {
    let src = concat!(
        "query same_director:\n",
        "  ?movie1 director [ is director of ?movie2 ]\n",
        "  ?movie2 != ?movie1\n",
    );
    let result = parse_ontology(src);
    assert!(!result.has_errors(), "parse errors: {:?}", result.errors());
    let q = result.ontology.unwrap().queries().into_iter().next().unwrap().clone();
    let sparql = to_sparql(&q, &[&q]).expect("sparql generates");
    eprintln!("=== SPARQL (hoisted filter) ===\n{sparql}");
    assert!(sparql.contains("?movie1 director"), "expected forward triple:\n{sparql}");
    assert!(sparql.contains("?movie2 director"), "expected inverse triple:\n{sparql}");
    assert!(sparql.contains("FILTER"), "expected the self-exclusion FILTER:\n{sparql}");
    assert!(sparql.contains("!="), "expected the != operator:\n{sparql}");
}
