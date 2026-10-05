//! A quantified rule must never be emitted as a live rule without its
//! quantifier: that rule would fire for every match of the rest of the body
//! (raft-bugs.md #4). Turtle output encodes quantifiers with `log:`/`list:`
//! builtins (`generate_n3_rules`).
//! Every commented-out rule is reported by `skipped_rules` for a warning.

use irukame::{TurtleGenerator, TurtleOptions};
use rowl::package::{load_package_from_memory, Package};
use std::collections::HashMap;

const SRC: &str = "\
concept Customer
concept Rental:
  has customer: Customer
  has distance: float
concept Flag

rule frequent_rider:
  match:
    ?c a Customer
    at least 2 ?r:
      ?r customer ?c
  then:
    ?c a Flag

rule idle:
  match:
    ?c a Customer
    none ?r:
      ?r customer ?c
  then:
    ?c a Flag

rule some_rentals:
  match:
    ?c a Customer
    between 1, 3 ?r [ a Rental ]:
      ?r customer ?c
  then:
    ?c a Flag

rule all_long:
  match:
    ?c a Customer
    all ?r [ customer ?c ]:
      ?r distance [ > 10.0 ]
  then:
    ?c a Flag

rule double_count:
  match:
    ?c a Customer
    at least 2 ?r:
      ?r customer ?c
      ?r distance ?d
  then:
    ?c a Flag

rule all_without_domain:
  match:
    ?c a Customer
    all ?r:
      ?r customer ?c
  then:
    ?c a Flag

rule tag_rental:
  match:
    ?r a Rental
  then:
    ?r a Rental
";

fn package() -> Package {
    let mut files = HashMap::new();
    files.insert(
        "package.dlf".to_string(),
        "package <http://example.org/test>:\n  dolfin_version \"1\"\n  version \"0.1.0\"\n".to_string(),
    );
    files.insert("a.dlf".to_string(), SRC.to_string());
    load_package_from_memory(&files).unwrap()
}

/// The lines of rule `name`, from its `# Rule:` line to the next blank line.
fn rule_block<'a>(out: &'a str, name: &str) -> Vec<&'a str> {
    let header = format!("# Rule: {name}");
    let block: Vec<&str> =
        out.lines().skip_while(|l| *l != header).take_while(|l| !l.is_empty()).collect();
    assert!(block.len() > 2, "rule {name} missing:\n{out}");
    block
}

fn is_live(block: &[&str]) -> bool {
    block[1..].iter().all(|l| !l.starts_with('#'))
}

fn skipped(generator: &TurtleGenerator) -> Vec<&str> {
    generator.skipped_rules().iter().map(|(name, _)| name.as_str()).collect()
}

#[test]
fn turtle_encodes_quantifiers() {
    let options = TurtleOptions { include_rules_as_comments: false, ..TurtleOptions::default() };
    let mut generator = TurtleGenerator::new(options);
    let ttl = generator.generate(&package()).unwrap();

    assert!(ttl.contains("@prefix log: <http://www.w3.org/2000/10/swap/log#> ."), "{ttl}");
    assert!(ttl.contains("@prefix list: <http://www.w3.org/2000/10/swap/list#> ."), "{ttl}");

    let count = rule_block(&ttl, "frequent_rider").join("\n");
    assert!(is_live(&rule_block(&ttl, "frequent_rider")), "{count}");
    assert!(count.contains("( ?r { ?r a:customer ?c . } ?_v"), "{count}");
    assert!(count.contains("log:collectAllIn ?_scope"), "{count}");
    assert!(count.contains("math:notLessThan 2 ."), "{count}");

    let none = rule_block(&ttl, "idle").join("\n");
    assert!(none.contains("?_scope log:notIncludes { ?r a:customer ?c . } ."), "{none}");

    let between = rule_block(&ttl, "some_rentals").join("\n");
    assert!(between.contains("{ ?r a a:Rental . ?r a:customer ?c . }"), "{between}");
    assert!(between.contains("math:notLessThan 1 .") && between.contains("math:notGreaterThan 3 ."));

    let all = rule_block(&ttl, "all_long").join("\n");
    assert!(all.contains("( { ?r a:customer ?c . } { ?r a:distance ?_v"), "{all}");
    assert!(all.contains("log:forAllIn ?_scope ."), "{all}");

    // No faithful encoding: commented out, with the reason, and reported.
    let double = rule_block(&ttl, "double_count");
    assert!(!is_live(&double));
    assert!(double[1].contains("once per binding of ?d"), "{double:?}");
    assert!(!is_live(&rule_block(&ttl, "all_without_domain")));
    assert_eq!(skipped(&generator), ["double_count", "all_without_domain"]);

    assert!(is_live(&rule_block(&ttl, "tag_rental")));
    assert!(!ttl.contains("span"), "Debug dump leaked:\n{ttl}");
}

#[test]
fn no_rules_mode_reports_nothing() {
    let mut generator = TurtleGenerator::new(TurtleOptions::default());
    generator.generate(&package()).unwrap();
    assert!(generator.skipped_rules().is_empty());
}
