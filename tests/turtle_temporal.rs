//! Temporal smart literals render to XSD-typed Turtle values, and the file's
//! `@locale` / `@timezone` directives are threaded into resolution.

use irukame::{TurtleGenerator, TurtleOptions};
use rowl::package::load_package_from_memory;
use std::collections::HashMap;

fn options() -> TurtleOptions {
    TurtleOptions {
        base_iri: "http://example.org/".to_string(),
        include_comments: false,
        include_rules_as_comments: false,
        include_queries_as_comments: false,
    }
}

fn generate(onto: &str) -> String {
    let mut files = HashMap::new();
    files.insert(
        "package.dlf".to_string(),
        "package <http://example.org/test>:\n  dolfin_version \"1\"\n  version \"0.1.0\"\n  author \"test\"\n".to_string(),
    );
    files.insert("onto.dlf".to_string(), onto.to_string());
    let package = load_package_from_memory(&files).expect("package should load");
    TurtleGenerator::new(options())
        .generate(&package)
        .expect("generation should succeed")
}

#[test]
fn natural_date_renders_xsd_date() {
    let ttl = generate("concept Person:\n  has birthDate: date\n\nfact bob a Person\n  birthDate date(June 1st 2026)\n");
    assert!(
        ttl.contains("\"2026-06-01\"^^xsd:date"),
        "expected xsd:date literal:\n{ttl}"
    );
}

#[test]
fn duration_renders_xsd_duration() {
    let ttl = generate("concept Task:\n  has span: duration\n\nfact t a Task\n  span duration(1y 6mo)\n");
    assert!(
        ttl.contains("\"P1Y6M\"^^xsd:duration"),
        "expected xsd:duration literal:\n{ttl}"
    );
}

#[test]
fn locale_directive_threads_into_render() {
    // With @locale d/m/y the numeric date resolves; without it, it would fall
    // back to a bare string. This proves the directive reaches render_literal.
    let ttl = generate("@locale d/m/y\n\nconcept Person:\n  has birthDate: date\n\nfact bob a Person\n  birthDate date(01/06/2026)\n");
    assert!(
        ttl.contains("\"2026-06-01\"^^xsd:date"),
        "expected @locale to resolve the numeric date:\n{ttl}"
    );
}

#[test]
fn timezone_directive_threads_into_render() {
    let ttl = generate("@timezone +02:00\n\nconcept Event:\n  has startsAt: time\n\nfact e a Event\n  startsAt time(14:30)\n");
    assert!(
        ttl.contains("\"14:30:00+02:00\"^^xsd:time"),
        "expected @timezone offset applied:\n{ttl}"
    );
}
