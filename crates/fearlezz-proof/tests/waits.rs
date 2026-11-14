//! Who may settle what waits, and how a weakening reads and writes.

use fearlezz_proof::waits::{Kind, Part, Role, Weakening};

#[test]
fn approving_needs_write_access_and_accepting_admin_rights() {
    assert!(Kind::Approve.allows(Role::of("write")));
    assert!(Kind::Approve.allows(Role::of("admin")));
    assert!(!Kind::Approve.allows(Role::of("read")));
    assert!(!Kind::Accept.allows(Role::of("write")));
    assert!(Kind::Accept.allows(Role::of("admin")));
}

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
