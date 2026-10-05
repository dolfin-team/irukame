//! The N3 that `rules_as_n3` emits has to survive retox's N3 lexer.
//!
//! It did not: every rule was prefixed with a bare `Rule: <name>` line, which
//! N3 has no syntax for. The first rule in a package therefore failed with
//! `Lex error at position N` (retox `rules/n3/lexer.rs`) — reported from the
//! Agrafe SPARQL tab, where the generated N3 is never shown, so the position
//! pointed at text nobody could read.

use irukame::rules_as_n3;
use rowl::package::load_package;
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn n3_for(name: &str) -> String {
    let pkg = load_package(fixture(name)).expect("package loads");
    rules_as_n3(&pkg).expect("n3 generates")
}

#[test]
fn rule_names_are_carried_as_comments() {
    let n3 = n3_for("inverse_usage");
    assert!(
        n3.contains("# Rule: played_together"),
        "rule name should survive as a comment:\n{n3}"
    );
    // Nothing may reintroduce a bare name line: N3 would not lex it.
    for line in n3.lines() {
        assert!(
            !line.trim_start().starts_with("Rule:"),
            "uncommented rule name line:\n{line}"
        );
    }
}
