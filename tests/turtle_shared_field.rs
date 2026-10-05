//! A field name shared by two concepts is one property IRI: its domain must be
//! the union of the concepts, never one `rdfs:domain` per concept (which makes
//! every instance of one concept a member of the other; BUGS.md #1).

use irukame::TurtleGenerator;
use rowl::package::load_package_from_memory;
use std::collections::HashMap;

const SRC: &str = "\
concept Veterinarian:
  has name: one string
  has patient: Animal

concept Animal:
  has name: one string
  has patient: any Veterinarian
  has tag: string

concept Clinic:
  has tag: int
";

#[test]
fn shared_field_has_union_domain() {
    let mut files = HashMap::new();
    files.insert(
        "package.dlf".to_string(),
        "package <http://example.org/test>:\n  dolfin_version \"1\"\n  version \"0.1.0\"\n".to_string(),
    );
    files.insert("a.dlf".to_string(), SRC.to_string());
    let ttl = TurtleGenerator::with_defaults().generate(&load_package_from_memory(&files).unwrap()).unwrap();

    let union = "rdfs:domain [ a owl:Class ; owl:unionOf ( a:Animal a:Veterinarian ) ]";
    assert!(!ttl.contains("rdfs:domain a:Veterinarian") && !ttl.contains("rdfs:domain a:Animal"), "{ttl}");
    assert_eq!(ttl.matches(union).count(), 4, "{ttl}");
    // Same range: kept; `one` in both: functional.
    assert!(ttl.contains("a:name rdf:type owl:DatatypeProperty\n  ; rdfs:domain [ a owl:Class ; owl:unionOf ( a:Animal a:Veterinarian ) ]\n  ; rdfs:range xsd:string\n  ; rdf:type owl:FunctionalProperty"), "{ttl}");
    // Differing object ranges: their union; `any` in one: not functional.
    let patient = "rdfs:range [ a owl:Class ; owl:unionOf ( a:Animal a:Veterinarian ) ]";
    assert_eq!(ttl.matches(patient).count(), 2, "{ttl}");
    // Differing datatype ranges: a datatype union.
    assert!(ttl.contains("rdfs:range [ a rdfs:Datatype ; owl:unionOf ( xsd:integer xsd:string ) ]"), "{ttl}");
    let patient_block = ttl.split("a:patient rdf:type").nth(1).unwrap();
    assert!(!patient_block[..patient_block.find("\n  .").unwrap()].contains("FunctionalProperty"), "{ttl}");
}
