//! Full-output snapshot tests for the Turtle generator.
//!
//! Each test captures the complete Turtle text for a fixture package.
//! On first run (or after `cargo insta review`), snapshots are written to
//! `tests/snapshots/`. Subsequent runs fail if the output diverges.
//!
//! To regenerate all snapshots after an intentional output change:
//!   INSTA_UPDATE=always cargo test -p irukame turtle_snapshots

use irukame::{TurtleGenerator, TurtleOptions};
use rowl::package::{load_package, load_package_from_memory};
use std::collections::HashMap;
use std::path::PathBuf;

fn gen_from_fixture(fixture: &str, with_comments: bool) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(fixture);
    let package = load_package(path).expect("fixture package should load");
    TurtleGenerator::new(TurtleOptions {
        base_iri: "http://example.org/".to_string(),
        include_comments: with_comments,
        include_rules_as_comments: with_comments,
        include_queries_as_comments: false,
    })
    .generate(&package)
    .expect("generation should succeed")
}

// ── glossary fixture ──────────────────────────────────────────────────────────

#[test]
fn snapshot_glossary_no_comments() {
    insta::assert_snapshot!(gen_from_fixture("glossary", false));
}

#[test]
fn snapshot_glossary_with_comments() {
    insta::assert_snapshot!(gen_from_fixture("glossary", true));
}

// ── multilingual fixture (#xx> comments + definition@xx= args) ────────────────

#[test]
fn snapshot_multilingual_with_comments() {
    insta::assert_snapshot!(gen_from_fixture("multilingual", true));
}

// ── axioms fixture ────────────────────────────────────────────────────────────

#[test]
fn snapshot_axioms_no_comments() {
    insta::assert_snapshot!(gen_from_fixture("axioms", false));
}

#[test]
fn snapshot_axioms_with_comments() {
    insta::assert_snapshot!(gen_from_fixture("axioms", true));
}

// ── uri_prefix fixture ────────────────────────────────────────────────────────

#[test]
fn snapshot_uri_prefix_no_comments() {
    insta::assert_snapshot!(gen_from_fixture("uri_prefix", false));
}

// ── in-memory package: external prefix aliasing ───────────────────────────────

#[test]
fn snapshot_in_memory_external_prefix() {
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
    let ttl = TurtleGenerator::new(TurtleOptions {
        base_iri: "http://example.org/".to_string(),
        include_comments: false,
        include_rules_as_comments: false,
        include_queries_as_comments: false,
    })
    .generate(&package)
    .expect("generation should succeed");
    insta::assert_snapshot!(ttl);
}
