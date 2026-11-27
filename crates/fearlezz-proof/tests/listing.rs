//! A repository page's listing: where each work's newest run stands and what waits on it,
//! worded by the host so the page only shows it.

use fearlezz_proof::events::{Latest, Listed, RunState, Share, Source, Standing, Work};

fn listed(
    latest: Option<RunState>,
    failed: u32,
    pictures: u32,
    notes: u32,
    weakened: bool,
) -> Listed {
    Listed {
        work: Work {
            repository: "owner/repo".into(),
            branch: "a".into(),
            pull: None,
        },
        path: "/owner/repo/work/a".into(),
        seq: 1,
        active: 1.0,
        latest: latest.map(|state| Latest {
            head: "h".into(),
            source: Source::Ci {
                job: "proof".into(),
                run_url: None,
                run_id: 1,
                run_attempt: 1,
                runner_name: "runner".into(),
                leg: None,
                planned: Vec::new(),
                workflow: None,
                gate: false,
            },
            state,
            failed,
        }),
        pictures,
        notes,
        weakened,
        standing: Standing::None,
        word: String::new(),
        waits: Vec::new(),
    }
    .worded()
}

#[test]
fn each_work_says_where_its_newest_run_stands() {
    let standing = |l: Listed| (l.standing, l.word);
    let done = Some(RunState::Done);
    assert_eq!(
        standing(listed(None, 0, 0, 0, false)),
        (Standing::None, "no run yet".into())
    );
    assert_eq!(
        standing(listed(Some(RunState::Running), 0, 0, 0, false)),
        (Standing::Running, "running".into())
    );
    assert_eq!(
        standing(listed(done, 2, 0, 0, false)),
        (Standing::Failed, "failed".into())
    );
    assert_eq!(
        standing(listed(done, 0, 0, 0, false)),
        (Standing::Passed, "passed".into())
    );
    assert_eq!(
        standing(listed(Some(RunState::Cancelled), 0, 0, 0, false)),
        (Standing::Cancelled, "stopped".into())
    );
    // Passed checks with pictures, notes or a weakening still open wait for a person.
    assert_eq!(
        standing(listed(done, 0, 0, 0, true)),
        (Standing::Waits, "waits for a person".into())
    );
    assert_eq!(standing(listed(done, 0, 2, 0, false)).0, Standing::Waits);
}

#[test]
fn each_work_says_what_waits_on_it() {
    assert_eq!(
        listed(None, 0, 1, 2, true).waits,
        [
            "1 picture to look at",
            "2 open notes",
            "a weakening to accept"
        ]
    );
    assert_eq!(
        listed(None, 0, 3, 1, false).waits,
        ["3 pictures to look at", "1 open note"]
    );
    assert_eq!(listed(None, 0, 0, 0, false).waits, Vec::<String>::new());
}

#[test]
fn a_share_reads_as_the_host_lists_it() {
    let share: Share = serde_json::from_value(
        serde_json::json!({"id": "s1", "made_by": "writer", "made_at": 5.0}),
    )
    .unwrap();
    assert_eq!(share.made_by, "writer");
}
