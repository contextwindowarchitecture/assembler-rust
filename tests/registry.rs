//! The registry's published digests, which test the RFC 8785 serializer (conformance/README.md, Registry): a
//! profile's digest leaves out its `evaluation`, and a route policy's covers all of it.

use std::path::Path;

use cwa_assembler::canonical::{sha256_hex, to_string};
use cwa_assembler::json::parse;
use serde_json::Value;

fn read(name: &str) -> Value {
    parse(&std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("vendor/cwa/conformance/registry").join(name)).unwrap()).unwrap()
}

#[test]
fn profile_and_route_policy_digests_match_the_lock() {
    let lock = read("lock.json");
    let mut checked = 0;
    for profile in read("profiles.json").as_array().unwrap() {
        let mut without = profile.clone();
        without.as_object_mut().unwrap().remove("evaluation");
        let entry = lock["profiles"].as_array().unwrap().iter()
            .find(|e| e["id"] == profile["id"] && e["version"] == profile["version"]).expect("a lock entry");
        assert_eq!(sha256_hex(to_string(&without).as_bytes()), entry["sha256"].as_str().unwrap(), "{}", profile["id"]);
        checked += 1;
    }
    for policy in read("route-policies.json").as_array().unwrap() {
        let entry = lock["route_policies"].as_array().unwrap().iter()
            .find(|e| e["route"] == policy["route"] && e["version"] == policy["version"]).expect("a lock entry");
        assert_eq!(sha256_hex(to_string(policy).as_bytes()), entry["sha256"].as_str().unwrap(), "{} {}", policy["route"], policy["version"]);
        checked += 1;
    }
    assert!(checked >= 10);
}
