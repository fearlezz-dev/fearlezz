//! How a weakening reads and writes.

use fearlezz_proof::floor::{Part, Weakening};

#[test]
fn a_weakening_reads_and_writes_its_part_by_name() {
    let w = Weakening {
        part: Part::Check("tests".into()),
        how: "it runs another command".into(),
    };
    let json = r#"{"check":"tests","how":"it runs another command"}"#;
    assert_eq!(serde_json::to_string(&w).unwrap(), json);
    assert_eq!(serde_json::from_str::<Weakening>(json).unwrap(), w);
    assert_eq!(
        w.to_string(),
        "check `tests` is weaker: it runs another command"
    );
}
