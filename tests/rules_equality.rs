//! `=` / `!=` in a rule: `math:equalTo` only holds between numbers (and
//! dates), so against a string `[ = "Bob" ]` the rule could never fire. Term
//! comparisons use `log:equalTo` / `log:notEqualTo`.

use irukame::{TurtleGenerator, TurtleOptions, rules_as_n3};
use rowl::package::load_package;
use std::path::PathBuf;

fn package() -> rowl::package::Package {
    load_package(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/equality")).expect("package loads")
}

#[test]
fn string_equality_uses_log_builtins() {
    let out = TurtleGenerator::new(TurtleOptions {
        base_iri: "http://example.org/".to_string(),
        include_comments: false,
        include_rules_as_comments: false,
        include_queries_as_comments: false,
    })
    .generate(&package())
    .unwrap();
    assert!(out.contains("@prefix log:"), "{out}");
    assert!(out.contains("math:equalTo \"18\"^^xsd:integer"), "numbers keep math:\n{out}");
    assert!(out.contains("log:equalTo \"Bob\""), "{out}");
    assert!(out.contains("log:notEqualTo \"Bob\""), "{out}");
    assert!(!out.contains("math:equalTo \"Bob\""), "{out}");
}

/// Against a variable, the choice follows the inferred range of the property.
#[test]
fn variable_equality_follows_inferred_type() {
    let out = rules_as_n3(&package()).unwrap();
    let rule = |name: &str| {
        let start = out.find(&format!("# Rule: {name}\n")).unwrap_or_else(|| panic!("{name} missing:\n{out}"));
        out[start..].split("\n\n").next().unwrap().to_string()
    };
    assert!(rule("r_var_num").contains("math:equalTo ?a"), "{}", rule("r_var_num"));
    assert!(rule("r_var_str").contains("log:equalTo ?n"), "{}", rule("r_var_str"));
    assert!(rule("r_var_iri").contains("log:notEqualTo ?f"), "{}", rule("r_var_iri"));
    // `Mass` is a dimension concept: its values are quantities, not resources.
    assert!(rule("r_var_qty").contains("dq:equalTo ?w"), "{}", rule("r_var_qty"));
    assert!(rule("r_var_len").contains("dq:greaterThan ?l"), "{}", rule("r_var_len"));
    assert!(out.contains("@prefix dq:"), "{out}");
}
