use super::model::{Id, Shape, Summary};
use super::{Index, summarize};
use sha2::Digest as _;
use std::fmt::Write as _;

fn term(summary: &Summary, id: Id) -> serde_json::Value {
    use serde_json::json;
    match &summary.nodes[id].shape {
        Shape::Atom(name) => json!(["atom", name]),
        Shape::Bytes(bytes) => json!(["bytes", bytes]),
        Shape::Integer(value) => json!(["integer", value]),
        Shape::Float(bits) => json!(["float", bits.to_string()]),
        Shape::List(items) => json!([
            "list",
            items
                .iter()
                .map(|id| term(summary, *id))
                .collect::<Vec<_>>()
        ]),
        Shape::Pair(a, b) => json!(["pair", term(summary, *a), term(summary, *b)]),
        Shape::Call { head, args } => json!([
            "call",
            term(summary, *head),
            args.as_ref()
                .map(|args| args.iter().map(|id| term(summary, *id)).collect::<Vec<_>>())
        ]),
    }
}

#[test]
fn ex2002_oracle_shapes() {
    let data: serde_json::Value = serde_json::from_str(include_str!(
        "../../../compatibility/duplicated-shapes.json"
    ))
    .unwrap();
    verify_shapes(data.as_array().unwrap());
}

#[test]
#[ignore = "development oracle campaign: QREDO_SHAPES=oracle.json"]
fn ex2002_external_shapes() {
    let data: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(std::env::var("QREDO_SHAPES").unwrap()).unwrap(),
    )
    .unwrap();
    verify_shapes(data["shapes"].as_array().unwrap());
}

fn verify_shapes(cases: &[serde_json::Value]) {
    for case in cases {
        let source = case["source"].as_str().unwrap();
        let tree = crate::ts_parser::parse(source).unwrap();
        let summary = summarize(&tree, source);
        assert!(summary.valid(), "{source}");
        let root = summary.root.unwrap();
        let mut index = Index::default();
        index.collect(&summary, 0, 0);
        let normalized = term(&summary, root);
        let identity: String = sha2::Sha256::digest(normalized.to_string().as_bytes())
            .iter()
            .fold(String::new(), |mut out, byte| {
                let _ = write!(out, "{byte:02x}");
                out
            });
        if identity != case["identity"].as_str().unwrap() {
            if let Some(expected) = case.get("term") {
                compare_terms(&normalized, expected, "$", source);
            }
            panic!(
                "EX2002.shape {}: {normalized}",
                source.lines().next().unwrap_or("")
            );
        }
        assert_eq!(
            index.identities.last().unwrap().mass as u64,
            case["mass"].as_u64().unwrap(),
            "EX2002.mass {source}"
        );
    }
}

fn compare_terms(
    actual: &serde_json::Value,
    expected: &serde_json::Value,
    path: &str,
    source: &str,
) {
    if actual == expected {
        return;
    }
    if let (Some(a), Some(b)) = (actual.as_array(), expected.as_array())
        && a.len() == b.len()
    {
        for (i, (a, b)) in a.iter().zip(b).enumerate() {
            compare_terms(a, b, &format!("{path}[{i}]"), source);
        }
    }
    panic!(
        "EX2002.shape {} {path}: actual={actual} expected={expected}",
        source.lines().next().unwrap_or("")
    );
}
