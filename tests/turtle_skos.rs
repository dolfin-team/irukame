//! Tests for SKOS triple emission in the Turtle generator.

use irukame::{TurtleGenerator, TurtleOptions};
use rowl::package::load_package;
use std::path::PathBuf;

fn fixture_path(fixture: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(fixture)
}

fn generate_fixture(fixture: &str) -> String {
    let package = load_package(fixture_path(fixture)).expect("fixture package should load");
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

fn run_generator() -> String {
    generate_fixture("glossary")
}

#[test]
fn skos_prefix_is_emitted() {
    let ttl = run_generator();
    assert!(
        ttl.contains("@prefix skos: <http://www.w3.org/2004/02/skos/core#> ."),
        "expected skos prefix:\n{ttl}"
    );
}

// ── prefLabel fallback: name when no annotation ───────────────────────────────

#[test]
fn concept_without_annotation_gets_name_as_pref_label() {
    let ttl = run_generator();
    assert!(
        ttl.contains("skos:prefLabel \"Department\""),
        "expected name as prefLabel for Department:\n{ttl}"
    );
}

// ── leading comment → untagged rdfs:comment ──────────────────────────────────

#[test]
fn concept_with_leading_comment_gets_rdfs_comment() {
    let ttl = run_generator();
    // A plain leading `# …` comment becomes an untagged rdfs:comment (per the
    // revised contract: leading comments carry rdfs:comment, not skos:definition).
    assert!(
        ttl.contains("rdfs:comment \"A person in the HR system.\""),
        "expected leading comment as rdfs:comment for Person:\n{ttl}"
    );
}

// ── annotation: label= overrides name ────────────────────────────────────────

#[test]
fn annotation_label_overrides_name() {
    let ttl = run_generator();
    assert!(
        ttl.contains("skos:prefLabel \"Employment Status\""),
        "expected annotation label for EmploymentStatus:\n{ttl}"
    );
    // The raw name should not appear as prefLabel
    assert!(
        !ttl.contains("skos:prefLabel \"EmploymentStatus\""),
        "EmploymentStatus name should not appear as prefLabel:\n{ttl}"
    );
}

// ── annotation: definition= ──────────────────────────────────────────────────

#[test]
fn annotation_definition_is_emitted() {
    let ttl = run_generator();
    assert!(
        ttl.contains("skos:definition \"The employment contract type of an employee.\""),
        "expected annotation definition for EmploymentStatus:\n{ttl}"
    );
}

// ── annotation: alt_label= ───────────────────────────────────────────────────

#[test]
fn annotation_alt_label_is_emitted() {
    let ttl = run_generator();
    assert!(
        ttl.contains("skos:altLabel \"Contract type\""),
        "expected altLabel for EmploymentStatus:\n{ttl}"
    );
}

// ── annotation: scope_note= ──────────────────────────────────────────────────

#[test]
fn annotation_scope_note_is_emitted() {
    let ttl = run_generator();
    assert!(
        ttl.contains("skos:scopeNote \"Only applies to direct employees, not contractors.\""),
        "expected scopeNote for EmploymentStatus:\n{ttl}"
    );
}

// ── leading comment on a has-field → rdfs:comment (no definition fallback) ─────

#[test]
fn has_field_leading_comment_becomes_rdfs_comment() {
    let ttl = run_generator();
    // `has employeeId` has a leading comment and no `definition=` annotation.
    // The leading comment becomes rdfs:comment; the old desc→definition fallback
    // is gone.
    assert!(
        ttl.contains("rdfs:comment \"The unique employee identifier assigned by HR.\""),
        "expected leading comment as rdfs:comment for employeeId:\n{ttl}"
    );
}

// ── annotation + adjacent description (no blank line) ────────────────────────

#[test]
fn annotation_adjacent_desc_emits_alt_label_and_rdfs_comment() {
    let ttl = run_generator();
    assert!(
        ttl.contains("skos:altLabel \"Rotor Aircraft\""),
        "expected altLabel for HelicopterNoBlanks:\n{ttl}"
    );
    // The adjacent description (annotation has no definition= arg) becomes
    // rdfs:comment, not skos:definition.
    assert!(
        ttl.contains("rdfs:comment \"Rotary-wing aircraft. Adjacent description, no blank line.\""),
        "expected rdfs:comment for HelicopterNoBlanks:\n{ttl}"
    );
}

// ── adjacent annotation must not leak alt_label to first has-property ────────

#[test]
fn annotation_adjacent_desc_alt_label_does_not_leak_to_has_field() {
    let ttl = run_generator();
    // serialNum has its own comment ("Unique aircraft identifier.") but no annotation.
    // The concept annotation's alt_label must not bleed onto it.
    let serial_block_start = ttl.find("hr:serialNum rdf:type").expect("serialNum block missing");
    let serial_block = &ttl[serial_block_start..serial_block_start + 300];
    assert!(
        !serial_block.contains("skos:altLabel"),
        "serialNum must not inherit concept alt_label:\n{serial_block}"
    );
}

// ── annotation + blank line + description ─────────────────────────────────────

#[test]
fn annotation_blank_line_desc_emits_alt_label_and_rdfs_comment() {
    let ttl = run_generator();
    assert!(
        ttl.contains("skos:altLabel \"Rotor Aircraft Blank\""),
        "expected altLabel for HelicopterWithBlank:\n{ttl}"
    );
    // The blank-line-separated description (annotation has no definition= arg)
    // becomes rdfs:comment, not skos:definition.
    assert!(
        ttl.contains("rdfs:comment \"Rotary-wing aircraft. Blank line before description.\""),
        "expected rdfs:comment for HelicopterWithBlank:\n{ttl}"
    );
}

// ── has field with annotation ─────────────────────────────────────────────────

#[test]
fn has_field_annotation_label_and_definition() {
    let ttl = run_generator();
    assert!(
        ttl.contains("skos:prefLabel \"Annual salary\""),
        "expected annotation label for salary field:\n{ttl}"
    );
    assert!(
        ttl.contains("skos:definition \"Gross annual salary in EUR.\""),
        "expected annotation definition for salary field:\n{ttl}"
    );
}

// ── multilingual: #xx> comments and definition@xx= args ───────────────────────
//
// The `multilingual` fixture's `Cat` concept exercises all four
// carrier/predicate/language combinations. It lives in its own fixture (not the
// glossary one) because 3 skos:definition on one concept would otherwise produce
// duplicate rows in the SPARQL-based glossary plugin, which has no DISTINCT.
//   (a) `#en>` + `#fr>` leading comments  → two rdfs:comment@lang
//   (b) a plain `# …` leading comment     → one untagged rdfs:comment
//   (c) `definition@en=` + `definition@fr=` → two skos:definition@lang
//   (d) a plain `definition=`             → one untagged skos:definition

#[test]
fn tagged_leading_comments_emit_rdfs_comment_per_language() {
    let ttl = generate_fixture("multilingual");
    // (a) tagged rdfs:comment, one per language
    assert!(
        ttl.contains("rdfs:comment \"A cat is a small carnivorous mammal.\"@en"),
        "expected @en rdfs:comment for Cat:\n{ttl}"
    );
    assert!(
        ttl.contains("rdfs:comment \"Un chat est un petit mammifère carnivore.\"@fr"),
        "expected @fr rdfs:comment for Cat:\n{ttl}"
    );
}

#[test]
fn untagged_leading_comment_emits_plain_rdfs_comment() {
    let ttl = generate_fixture("multilingual");
    // (b) untagged rdfs:comment, no @lang suffix
    assert!(
        ttl.contains("rdfs:comment \"A plain untagged remark about cats.\"\n"),
        "expected untagged rdfs:comment for Cat:\n{ttl}"
    );
}

#[test]
fn tagged_definitions_emit_skos_definition_per_language() {
    let ttl = generate_fixture("multilingual");
    // (c) tagged skos:definition, one per language
    assert!(
        ttl.contains("skos:definition \"A domesticated feline kept as a companion animal.\"@en"),
        "expected @en skos:definition for Cat:\n{ttl}"
    );
    assert!(
        ttl.contains(
            "skos:definition \"Un félin domestique gardé comme animal de compagnie.\"@fr"
        ),
        "expected @fr skos:definition for Cat:\n{ttl}"
    );
}

#[test]
fn untagged_definition_emits_plain_skos_definition() {
    let ttl = generate_fixture("multilingual");
    // (d) untagged skos:definition, no @lang suffix
    assert!(
        ttl.contains("skos:definition \"An untagged feline definition.\"\n"),
        "expected untagged skos:definition for Cat:\n{ttl}"
    );
}
