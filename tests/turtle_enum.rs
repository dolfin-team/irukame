//! `one of` members in Turtle: every reference must hit the declared
//! `Owner_Member` individual, and a member's `[key value]` block is emitted.

use irukame::{TurtleGenerator, TurtleOptions};
use rowl::package::load_package_from_memory;
use std::collections::HashMap;

const MANIFEST: &str = r#"package battery:
  dolfin_version "1"
  version "0.1.0"
  author "test"
"#;

const CHEMISTRY: &str = r#"concept Chemistry:
  has key code: string
  one of:
    NMC [code "NMC"]
    LFP [code "LFP"]

concept BatteryModel:
  has chemistry: one Chemistry
  has variant: optional BatteryModel

concept NmcModel:
  sub BatteryModel

fact m1 a BatteryModel
  chemistry NMC
  variant [
    chemistry LFP
  ]

rule nmc:
  match:
    ?m chemistry NMC
  then:
    ?m a NmcModel
"#;

fn generate(files: HashMap<&str, &str>) -> (String, String) {
    let owned: HashMap<String, String> =
        files.into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let package = load_package_from_memory(&owned).expect("package load failed");
    let mut generator = TurtleGenerator::new(TurtleOptions {
        base_iri: "http://example.org/".to_string(),
        include_comments: false,
        include_rules_as_comments: false,
        include_queries_as_comments: false,
    });
    let ttl = generator.generate(&package).expect("generation failed");
    let n3 = generator.generate_n3_rules(&package).expect("n3 generation failed");
    (ttl, n3)
}

#[test]
fn enum_member_references_use_declared_individual_iri() {
    let (ttl, n3) = generate(HashMap::from([
        ("package.dlf", MANIFEST),
        ("chem.dlf", CHEMISTRY),
        (
            "other.dlf",
            "fact m2 a chem.BatteryModel\n  chem.chemistry chem.LFP\n",
        ),
    ]));
    assert!(ttl.contains("chem:Chemistry_NMC rdf:type owl:NamedIndividual"), "{ttl}");
    // fact value, anonymous block value, cross-file qualified value, rule body
    assert!(ttl.contains("chem:m1 chem:chemistry chem:Chemistry_NMC ."), "{ttl}");
    assert!(ttl.contains("chem:chemistry chem:Chemistry_LFP ."), "{ttl}");
    assert!(ttl.contains("other:m2 chem:chemistry chem:Chemistry_LFP ."), "{ttl}");
    assert!(n3.contains("chem:Chemistry_NMC"), "{n3}");
    assert!(!ttl.contains("chem:NMC ") && !ttl.contains("chem:LFP "), "{ttl}");
    assert!(!n3.contains("chem:NMC "), "{n3}");
}

#[test]
fn enum_member_key_values_are_emitted() {
    let (ttl, _) =
        generate(HashMap::from([("package.dlf", MANIFEST), ("chem.dlf", CHEMISTRY)]));
    assert!(ttl.contains("chem:Chemistry_NMC chem:code \"NMC\""), "{ttl}");
    assert!(ttl.contains("chem:Chemistry_LFP chem:code \"LFP\""), "{ttl}");
}
