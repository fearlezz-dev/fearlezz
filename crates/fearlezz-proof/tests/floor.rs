//! How a weakening reads and writes, and how an accept matches it.

use fearlezz_proof::floor::{Part, Weakening};
use fearlezz_proof::review::Accept;

fn command() -> Weakening {
    Weakening {
        part: Part::Check("tests".into()),
        how: "it runs another command".into(),
        was: None,
        now: None,
    }
}

#[test]
fn a_weakening_reads_and_writes_its_part_by_name() {
    let w = command();
    let json = r#"{"check":"tests","how":"it runs another command"}"#;
    assert_eq!(serde_json::to_string(&w).unwrap(), json);
    assert_eq!(serde_json::from_str::<Weakening>(json).unwrap(), w);
    assert_eq!(
        w.to_string(),
        "check `tests` is weaker: it runs another command"
    );
}

#[test]
fn a_weakening_carries_what_the_part_was_and_is_now() {
    let w = Weakening {
        was: Some("[check.tests]\nrun = \"cargo test\"\n".into()),
        now: Some("[check.tests]\nrun = \"true\"\n".into()),
        ..command()
    };
    let json = serde_json::to_string(&w).unwrap();
    assert_eq!(
        json,
        r#"{"check":"tests","how":"it runs another command","was":"[check.tests]\nrun = \"cargo test\"\n","now":"[check.tests]\nrun = \"true\"\n"}"#
    );
    assert_eq!(serde_json::from_str::<Weakening>(&json).unwrap(), w);
    assert_eq!(
        w.to_string(),
        "check `tests` is weaker: it runs another command"
    );
}

#[test]
fn an_accept_covers_the_weakenings_the_host_kept_as_they_were_sent() {
    // The host keeps the weakenings a run's proof showed and an accept takes them from there,
    // so a run of the same head asks with the same text and is covered.
    let w = Weakening {
        was: Some("[check.tests]\nrun = \"cargo test\"\n".into()),
        now: Some("[check.tests]\nrun = \"true\"\n".into()),
        ..command()
    };
    let kept: Vec<Weakening> =
        serde_json::from_str(&serde_json::to_string(&vec![w.clone()]).unwrap()).unwrap();
    let accept = Accept {
        head: "abc".into(),
        weakened: kept,
        by: "admin".into(),
        at: 1.0,
    };
    assert!(accept.covers("abc", std::slice::from_ref(&w)));
    let other = Weakening {
        now: Some("[check.tests]\nrun = \"echo\"\n".into()),
        ..w
    };
    assert!(!accept.covers("abc", &[other]));
}
