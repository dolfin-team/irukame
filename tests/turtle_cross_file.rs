//! A fully qualified reference to another file (`people.customers.Customer`)
//! uses that file's declared prefix, not one built from the whole name.

use irukame::{TurtleGenerator, TurtleOptions};
use rowl::package::load_package_from_memory;
use std::collections::HashMap;

#[test]
fn qualified_cross_file_ref_uses_file_prefix() {
    let files: HashMap<String, String> = [
        (
            "package.dlf",
            "package <http://example.org/bikes>:\n  dolfin_version \"1\"\n  version \"0.1.0\"\n",
        ),
        ("people/customers.dlf", "concept Customer\n\nfact alice a Customer\n"),
        (
            "rentals.dlf",
            "concept Rental:\n  has customer: one people.customers.Customer\n\n\
             fact r1 a Rental\n  customer people.customers.alice\n",
        ),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();

    let package = load_package_from_memory(&files).unwrap();
    let ttl = TurtleGenerator::new(TurtleOptions {
        base_iri: String::new(),
        include_comments: false,
        include_rules_as_comments: false,
        include_queries_as_comments: false,
    })
    .generate(&package)
    .unwrap();

    assert!(ttl.contains("rdfs:range people_customers:Customer"), "{ttl}");
    assert!(ttl.contains("rentals:customer people_customers:alice"), "{ttl}");
    assert!(!ttl.contains("people_customers_Customer:"), "{ttl}");
}

/// A bare fact id or enum member declared only in another file resolves to
/// that file (analyzer rule: this file first, else the only declaration).
#[test]
fn bare_cross_file_fact_and_enum_member() {
    let files: HashMap<String, String> = [
        (
            "package.dlf",
            "package <http://example.org/bikes>:\n  dolfin_version \"1\"\n  version \"0.1.0\"\n",
        ),
        (
            "people/customers.dlf",
            "concept Plan:\n  one of:\n    Monthly\n    Yearly\n\nconcept Customer\n\nfact alice a Customer\n",
        ),
        (
            "rentals.dlf",
            "concept Rental:\n  has customer: one people.customers.Customer\n  has plan: one people.customers.Plan\n\n\
             fact r1 a Rental\n  customer alice\n  plan Yearly\n\n\
             fact local a Rental\n  customer r1\n",
        ),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();

    let package = load_package_from_memory(&files).unwrap();
    let ttl = TurtleGenerator::new(TurtleOptions {
        base_iri: String::new(),
        include_comments: false,
        include_rules_as_comments: false,
        include_queries_as_comments: false,
    })
    .generate(&package)
    .unwrap();

    assert!(ttl.contains("rentals:r1 rentals:customer people_customers:alice ."), "{ttl}");
    assert!(ttl.contains("rentals:r1 rentals:plan people_customers:Plan_Yearly ."), "{ttl}");
    assert!(ttl.contains("rentals:local rentals:customer rentals:r1 ."), "{ttl}");
    assert!(!ttl.contains("rentals:alice"), "{ttl}");
}
