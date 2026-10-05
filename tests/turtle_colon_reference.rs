//! A `:Name` fact reference is the package-default prefix, as in Turtle: it
//! is not the fact `Name` of the current file. The bare `Name` is.

use irukame::{TurtleGenerator, TurtleOptions};
use rowl::package::load_package_from_memory;
use std::collections::HashMap;

fn turtle(files: &[(&str, &str)]) -> String {
    let files: HashMap<String, String> =
        files.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let package = load_package_from_memory(&files).unwrap();
    TurtleGenerator::new(TurtleOptions {
        base_iri: String::new(),
        include_comments: false,
        include_rules_as_comments: false,
        include_queries_as_comments: false,
    })
    .generate(&package)
    .unwrap()
}

const PACKAGE: &str =
    "package <http://happy-paws-clinic>:\n  dolfin_version \"1\"\n  version \"0.1.0\"\n";
const MODEL: &str = "concept Owner:\n  has firstName: one string\n\n\
                     concept Dog:\n  has owner: one Owner\n";

fn data(owner: &str) -> String {
    format!(
        "fact JohnSmith a model.Owner\n  firstName \"John\"\n\n\
         fact Biscuit a model.Dog\n  owner {owner}\n"
    )
}

#[test]
fn colon_reference_is_package_namespace() {
    let ttl = turtle(&[("package.dlf", PACKAGE), ("model.dlf", MODEL), ("data.dlf", &data(":JohnSmith"))]);
    assert!(ttl.contains("@prefix : <http://happy-paws-clinic#> ."), "{ttl}");
    assert!(ttl.contains("data:Biscuit model:owner :JohnSmith ."), "{ttl}");
}

#[test]
fn bare_reference_is_this_file() {
    let ttl = turtle(&[("package.dlf", PACKAGE), ("model.dlf", MODEL), ("data.dlf", &data("JohnSmith"))]);
    assert!(ttl.contains("data:Biscuit model:owner data:JohnSmith ."), "{ttl}");
    assert!(ttl.contains("data:JohnSmith model:firstName \"John\" ."), "{ttl}");
    assert!(ttl.contains("data:JohnSmith rdf:type owl:NamedIndividual\n  , model:Owner"), "{ttl}");
}
