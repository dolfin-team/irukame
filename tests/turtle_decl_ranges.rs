//! `generate_with_decl_ranges`: ranges are byte offsets into the final output.

use irukame::{DeclKey, DeclKind, TurtleGenerator, TurtleOptions};
use rowl::package::{load_package, load_package_from_memory, Package};
use std::collections::HashMap;
use std::ops::Range;
use std::path::PathBuf;

fn opts(comments: bool) -> TurtleOptions {
    TurtleOptions {
        base_iri: "http://example.org/".to_string(),
        include_comments: comments,
        include_rules_as_comments: comments,
        include_queries_as_comments: false,
    }
}

fn check(package: &Package, comments: bool) -> (String, Vec<(DeclKey, Range<usize>)>) {
    let plain = TurtleGenerator::new(opts(comments)).generate(package).unwrap();
    let (ttl, ranges) =
        TurtleGenerator::new(opts(comments)).generate_with_decl_ranges(package).unwrap();
    assert_eq!(plain, ttl, "generate must equal generate_with_decl_ranges().0");

    let mut prev_end = 0;
    for (key, r) in &ranges {
        assert!(r.start < r.end && r.end <= ttl.len(), "{key:?} {r:?}");
        assert!(r.start >= prev_end, "ranges must ascend without overlap at {key:?}");
        prev_end = r.end;
        assert!(ttl.is_char_boundary(r.start) && ttl.is_char_boundary(r.end));
    }
    // Every concept's first range holds its `rdf:type rdfs:Class` line.
    for (i, (key, r)) in ranges.iter().enumerate() {
        if key.kind == DeclKind::Concept && !ranges[..i].iter().any(|(k, _)| k == key) {
            let text = &ttl[r.clone()];
            assert!(text.contains(" rdf:type rdfs:Class"), "{key:?}: {text}");
            assert!(text.contains(&key.name), "{key:?}: {text}");
        }
    }
    (ttl, ranges)
}

#[test]
fn fixture_ranges() {
    for fixture in ["glossary", "multilingual", "axioms", "uri_prefix", "facts", "inverse_usage"] {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(fixture);
        let package = load_package(path).expect("fixture loads");
        for comments in [false, true] {
            let (_, ranges) = check(&package, comments);
            assert!(
                ranges.iter().any(|(k, _)| k.kind == DeclKind::Concept),
                "{fixture}: expected at least one concept range"
            );
        }
    }
}

#[test]
fn facts_and_properties_have_ranges() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/facts");
    let package = load_package(path).unwrap();
    let (ttl, ranges) = check(&package, true);
    let rex = ranges.iter().find(|(k, _)| k.kind == DeclKind::Fact && k.name == "rex").unwrap();
    assert!(ttl[rex.1.clone()].contains("Rex"));
    assert!(ranges.iter().any(|(k, _)| k.kind == DeclKind::Property && k.name == "owner"));
}

#[test]
fn fields_and_enum_variants() {
    let mut files = HashMap::new();
    files.insert(
        "package.dlf".to_string(),
        "package <http://example.org/test>:\n  dolfin_version \"1\"\n  version \"0.1.0\"\n  author \"test\"\n".to_string(),
    );
    files.insert(
        "a.dlf".to_string(),
        "concept Colour:\n  has hex: string\n  one of:\n    Red\n    Green\n\nconcept Other\n".to_string(),
    );
    let package = load_package_from_memory(&files).unwrap();
    let (ttl, ranges) = check(&package, false);
    let find = |kind: DeclKind, name: &str| {
        ranges.iter().find(|(k, _)| k.kind == kind && k.name == name).map(|(_, r)| &ttl[r.clone()])
    };
    let owner = || "Colour".to_string();
    assert!(find(DeclKind::Field { owner: owner() }, "hex").is_some());
    assert!(find(DeclKind::EnumVariant { owner: owner() }, "Red").unwrap().contains("Colour_Red"));
    assert!(find(DeclKind::EnumVariant { owner: owner() }, "Green").is_some());
    // Enumeration's owl:equivalentClass is a second Concept range.
    assert_eq!(ranges.iter().filter(|(k, _)| k.kind == DeclKind::Concept && k.name == "Colour").count(), 2);
    assert!(find(DeclKind::Concept, "Other").is_some());
}

/// A rule's range spans its `# Rule:` header and N3 body, live or commented out.
#[test]
fn rules_have_ranges() {
    let mut files = HashMap::new();
    files.insert(
        "package.dlf".to_string(),
        "package <http://example.org/test>:\n  dolfin_version \"1\"\n  version \"0.1.0\"\n".to_string(),
    );
    files.insert(
        "a.dlf".to_string(),
        "\
# Café à côté
concept Customer
concept Rental:
  has customer: Customer
  has distance: float
concept Flag

rule tag_rental:
  match:
    ?r a Rental
  then:
    ?r a Flag

rule double_count:
  match:
    ?c a Customer
    at least 2 ?r:
      ?r customer ?c
      ?r distance ?d
  then:
    ?c a Flag
"
        .to_string(),
    );
    let package = load_package_from_memory(&files).unwrap();
    for comments in [false, true] {
        let (ttl, ranges) = check(&package, comments);
        let rule = |name: &str| {
            let key = DeclKey { file: "a.dlf".into(), kind: DeclKind::Rule, name: name.into() };
            ranges.iter().find(|(k, _)| *k == key).map(|(_, r)| &ttl[r.clone()]).unwrap()
        };
        let live = rule("tag_rental");
        assert!(live.starts_with("# Rule: tag_rental\n"), "{live}");
        assert!(live.contains("} ."), "{live}");
        assert_eq!(live.lines().nth(1) == Some("{"), !comments, "{live}");
        let skipped = rule("double_count");
        assert!(skipped.starts_with("# Rule: double_count\n# Not emitted as a live rule"), "{skipped}");
        assert!(skipped.contains("# }  => {"), "{skipped}");
        assert!(!skipped.contains("tag_rental"), "{skipped}");
    }
}

/// Multi-file package: the Turtle text (and so every range) must not depend
/// on `Package.ontologies` HashMap order, which is reseeded on every load.
/// Agrafe's "Reveal in turtle export" pairs ranges from one compile with the
/// text of another.
#[test]
fn multi_file_output_is_stable_across_loads() {
    let files: HashMap<String, String> = [
        ("package.dlf", "package <http://example.org/ada>:\n  dolfin_version \"1\"\n  version \"0.1.0\"\n"),
        ("comp.dlf", "\
concept Dog:
  has name: string
concept Unassigned

rule flag_unassigned:
  match:
    ?d a Dog
    none ?n:
      ?d name ?n
  then:
    ?d a Unassigned

rule many_names:
  match:
    ?d a Dog
    at least 2 ?n:
      ?d name ?n
  then:
    ?d a Unassigned

rule nested_only:
  match:
    ?d a Dog
  then:
    match:
      ?d name ?n
    then:
      ?d a Unassigned
"),
        ("foo/a/b.dlf", "concept FooB:\n  has y: string\nconcept FooC\n"),
        ("foo/inzip.dlf", "concept InZip:\n  has z: string\nconcept InZip2\n"),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();

    let mut first: Option<String> = None;
    for _ in 0..20 {
        let package = load_package_from_memory(&files).unwrap();
        let (ttl, ranges) = check(&package, true);
        let slice = |kind: DeclKind, name: &str| {
            let key = DeclKey { file: "comp.dlf".into(), kind, name: name.into() };
            ranges.iter().find(|(k, _)| *k == key).map(|(_, r)| &ttl[r.clone()]).unwrap()
        };
        for rule in ["flag_unassigned", "many_names"] {
            let text = slice(DeclKind::Rule, rule);
            assert!(text.starts_with(&format!("# Rule: {rule}\n")), "{rule}: {text}");
        }
        let nested = slice(DeclKind::Rule, "nested_only");
        assert!(nested.starts_with("# Rule: nested_only"), "{nested}");
        let dog = slice(DeclKind::Concept, "Dog");
        assert!(dog.starts_with("# Concept: Dog"), "{dog}");
        match &first {
            None => first = Some(ttl),
            Some(f) => assert_eq!(f, &ttl, "Turtle text changed between loads"),
        }
    }
}
