//! Where review of a head stands: which pictures wait for a verdict, whether its weakenings
//! wait for an admin's accept, and what a verdict follows from run to run.

use std::collections::{BTreeMap, BTreeSet};

use fearlezz_proof::events::{Run, RunCheck, RunState, Source};
use fearlezz_proof::floor::{Part, Weakening};
use fearlezz_proof::look::{Change, Item, Look};
use fearlezz_proof::media::{Comparison, Mark, Marks, Pair, Verdict as Likeness};
use fearlezz_proof::notes::{Note, NoteState, Region, Side};
use fearlezz_proof::review::{self, Accept, Actions, Missing, State, Verdict};
use fearlezz_proof::{Check, Proof, Ran};

fn still(name: &str, change: Change) -> Item {
    Item {
        name: name.into(),
        kind: "still".into(),
        change,
        summary: String::new(),
        before: (change != Change::New).then(|| format!("looks/s/before/{name}")),
        after: (change != Change::Gone).then(|| format!("looks/s/after/{name}")),
        comparison: None,
        flaky: false,
        diff: None,
        webp: None,
        marks: None,
        before_marks: None,
    }
}

fn pair(state: usize, px: usize) -> Pair {
    Pair {
        before: state,
        after: state,
        before_frame: 0,
        after_frame: 0,
        before_at: 0.0,
        after_at: 0.0,
        px,
        boxes: Vec::new(),
    }
}

fn film(changed: &[usize]) -> Item {
    Item {
        name: "checkout.webm".into(),
        kind: "film".into(),
        change: Change::Changed,
        summary: String::new(),
        before: Some("looks/s/before/checkout.webm".into()),
        after: Some("looks/s/after/checkout.webm".into()),
        comparison: Some(Comparison {
            verdict: Likeness::Changed,
            still: false,
            size_before: (640, 400),
            size_after: (640, 400),
            total_before: 3.0,
            total_after: 3.0,
            max_shift: 0.0,
            max_shift_at: None,
            marks_changed: false,
            timeline: Vec::new(),
            states_before: 3,
            states_after: 3,
            moments: Vec::new(),
            pairs: (0..3)
                .map(|s| pair(s, if changed.contains(&s) { 100 } else { 0 }))
                .collect(),
        }),
        flaky: false,
        diff: None,
        webp: None,
        marks: None,
        before_marks: None,
    }
}

/// The content key of every picture of a look, its files hashed to `sha`.
fn keys(items: &[Item], sha: &str) -> BTreeMap<String, String> {
    items
        .iter()
        .filter_map(|i| Some((i.name.clone(), review::key(i, |_| Some(sha.to_owned()))?)))
        .collect()
}

fn entry(id: &str, outcome: Ran) -> Check {
    Check {
        id: id.into(),
        platform: None,
        key: None,
        reused: None,
        run: "true".into(),
        stage: "working".into(),
        outcome,
        seconds: 1.0,
        exit_code: None,
        tests: None,
        cases: Vec::new(),
        findings: Vec::new(),
        findings_elsewhere: 0,
        output: None,
        failure: (outcome == Ran::Failed).then(|| fearlezz_proof::Item {
            what: "w".into(),
            why: "y".into(),
            fix: "f".into(),
        }),
        not_run: None,
        unstable: false,
        red: None,
        log: None,
    }
}

/// A CI run of `head` whose check `screens` showed `items`, and its proof when done.
fn run(id: &str, head: &str, state: RunState, items: &[Item]) -> (Run, Option<Proof>) {
    let look = Look {
        check: "screens".into(),
        base_commit: "base".into(),
        base_result: "passed".into(),
        no_before: None,
        note: None,
        items: items.to_vec(),
        dir: std::path::PathBuf::default(),
    };
    let check = RunCheck {
        id: "screens".into(),
        started: Some(1.0),
        entry: Some(entry("screens", Ran::Passed)),
        keys: keys(items, id),
        since: BTreeMap::new(),
        look: Some(look.clone()),
    };
    let run = Run {
        id: id.into(),
        head: head.into(),
        base: "main".into(),
        source: Source::Ci {
            job: "proof".into(),
            run_url: None,
            run_id: 1,
            run_attempt: 1,
            runner_name: "runner".into(),
            leg: None,
            planned: Vec::new(),
        },
        state,
        opened: 1.0,
        checks: vec![check],
    };
    let proof = (state == RunState::Done).then(|| Proof {
        checks: vec![entry("screens", Ran::Passed)],
        looks: vec![look],
        ..Proof::new(head.into(), head.into(), "main".into())
    });
    (run, proof)
}

fn verdict(picture: &str, key: &str, head: &str) -> Verdict {
    Verdict {
        check: "screens".into(),
        picture: picture.into(),
        key: key.into(),
        by: "writer".into(),
        at: 5.0,
        run: "r0".into(),
        head: head.into(),
    }
}

#[test]
fn a_head_without_a_run_waits_for_ci() {
    let review = review::review("aaa", &[], &Actions::default(), &[], &[], None);
    assert_eq!(review.state, State::Ci);
}

#[test]
fn a_running_run_of_the_head_is_running_and_another_heads_runs_do_not_count() {
    let runs = [
        run("r2", "bbb", RunState::Running, &[]),
        run("r1", "aaa", RunState::Done, &[]),
    ];
    assert_eq!(
        review::review("bbb", &runs, &Actions::default(), &[], &[], None).state,
        State::Running
    );
    assert_eq!(
        review::review("aaa", &runs, &Actions::default(), &[], &[], None).state,
        State::Approved
    );
}

#[test]
fn a_failed_proof_fails_the_review() {
    let (r, proof) = run("r1", "aaa", RunState::Done, &[]);
    let mut proof = proof.unwrap();
    proof.checks.push(entry("tests", Ran::Failed));
    let review = review::review(
        "aaa",
        &[(r, Some(proof))],
        &Actions::default(),
        &[],
        &[],
        None,
    );
    assert_eq!(review.state, State::Failed);
}

#[test]
fn changed_new_and_gone_pictures_wait_and_flaky_and_unchanged_ones_do_not() {
    let mut flaky = still("menu.png", Change::Changed);
    flaky.flaky = true;
    let items = vec![
        still("card.png", Change::Changed),
        still("new.png", Change::New),
        still("gone.png", Change::Gone),
        still("same.png", Change::Same),
        flaky,
    ];
    let runs = [run("r1", "aaa", RunState::Done, &items)];
    let review = review::review("aaa", &runs, &Actions::default(), &[], &[], None);
    assert_eq!(review.state, State::Waits);
    let waiting: Vec<&str> = review.waiting().map(|p| p.name.as_str()).collect();
    assert_eq!(waiting, ["card.png", "new.png", "gone.png"]);
}

#[test]
fn every_picture_looking_right_approves_the_head() {
    let runs = [run(
        "r1",
        "aaa",
        RunState::Done,
        &[still("card.png", Change::Changed)],
    )];
    let key = runs[0].0.checks[0].keys["card.png"].clone();
    let review = review::review(
        "aaa",
        &runs,
        &Actions::default(),
        &[verdict("card.png", &key, "aaa")],
        &[],
        None,
    );
    assert_eq!(review.state, State::Approved);
    assert_eq!(review.pictures[0].verdict.as_ref().unwrap().by, "writer");
    assert_eq!(review.title.read, "Everything looks right at aaa");
}

#[test]
fn an_approved_head_with_nothing_to_review_says_so_and_a_stored_review_gets_its_title() {
    let runs = [run("r1", "aaa", RunState::Done, &[])];
    let review = review::review("aaa", &runs, &Actions::default(), &[], &[], None);
    assert_eq!(review.state, State::Approved);
    assert_eq!(review.title.read, "Proven at aaa, with nothing to review");
    assert_eq!(review.title.admin, review.title.read);
    // A review stored before the host sent its title reads without one, and gets it again.
    let mut stored = serde_json::to_value(&review).unwrap();
    stored.as_object_mut().unwrap().remove("title");
    let read: review::Review = serde_json::from_value(stored).unwrap();
    assert_eq!(read.title.read, "");
    assert_eq!(read.titled(), review);
}

#[test]
fn the_viewer_reads_as_the_host_answers_it() {
    use fearlezz_proof::review::{Role, Viewer};
    let signed = Viewer::Signed {
        login: "writer".into(),
        role: Role::Write,
    };
    assert_eq!(
        serde_json::to_value(&signed).unwrap(),
        serde_json::json!({"login": "writer", "role": "write"})
    );
    let out: Viewer = serde_json::from_value(serde_json::json!({"login": null})).unwrap();
    assert_eq!(out, Viewer::Out { login: () });
    for role in [Role::Less, Role::Write, Role::Admin] {
        assert_eq!(serde_json::to_value(role).unwrap(), role.word());
    }
}

#[test]
fn a_verdict_follows_the_pictures_content_across_heads() {
    // The same bytes at a later head keep the verdict; other bytes lose it.
    let same = run(
        "r1",
        "bbb",
        RunState::Done,
        &[still("card.png", Change::Changed)],
    );
    let given = verdict("card.png", &same.0.checks[0].keys["card.png"], "aaa");
    let review = review::review(
        "bbb",
        &[same],
        &Actions::default(),
        std::slice::from_ref(&given),
        &[],
        None,
    );
    assert_eq!(review.state, State::Approved);
    assert_eq!(review.pictures[0].verdict.as_ref().unwrap().head, "aaa");

    let other = run(
        "r2",
        "ccc",
        RunState::Done,
        &[still("card.png", Change::Changed)],
    );
    let review = review::review("ccc", &[other], &Actions::default(), &[given], &[], None);
    assert_eq!(review.state, State::Waits);
}

#[test]
fn a_file_whose_bytes_are_not_known_yet_has_no_key() {
    assert_eq!(
        review::key(&still("card.png", Change::Changed), |_| None),
        None
    );
}

#[test]
fn a_new_or_gone_film_is_keyed_by_its_bytes() {
    // Nothing compared it, so another recording is another film: its verdict never carries over.
    for change in [Change::New, Change::Gone] {
        let mut item = film(&[]);
        item.change = change;
        item.comparison = None;
        if change == Change::New {
            item.before = None;
        } else {
            item.after = None;
        }
        let a = review::key(&item, |_| Some("x".into())).unwrap();
        let b = review::key(&item, |_| Some("y".into())).unwrap();
        assert_ne!(a, b, "{change:?}: another recording");
        assert_eq!(
            review::key(&item, |_| None),
            None,
            "{change:?}: bytes not known yet"
        );
    }
}

fn gone(check: &str) -> Weakening {
    Weakening {
        part: Part::Check(check.into()),
        hows: vec!["it is gone".into()],
        was: None,
        now: None,
    }
}

fn accept(head: &str, weakened: &[Weakening]) -> Accept {
    Accept {
        head: head.into(),
        weakened: weakened.to_vec(),
        by: "admin".into(),
        at: 9.0,
    }
}

#[test]
fn a_weakening_fails_the_review_until_the_hosts_accept_covers_it() {
    let (r, proof) = run("r1", "aaa", RunState::Done, &[]);
    let mut proof = proof.unwrap();
    proof.weakened = vec![gone("tests")];
    let runs = [(r.clone(), Some(proof.clone()))];
    let review = review::review("aaa", &runs, &Actions::default(), &[], &[], None);
    assert_eq!(review.state, State::Failed);
    assert_eq!(
        review.title(),
        "The proof of aaa failed: what it weakens waits for an admin's accept"
    );
    // The page shows the line its viewer can act on: an admin's accept is theirs to give.
    assert_eq!(review.title.read, review.title());
    assert_eq!(review.title.write, review.title());
    assert_eq!(
        review.title.admin,
        "The proof of aaa failed: what it weakens waits for your accept"
    );
    assert!(
        review.summary().contains("until an admin accepts it"),
        "{}",
        review.summary()
    );

    // The rerun's proof names the accept; the host holds it to its own.
    proof.accepted = Some(accept("aaa", &proof.weakened));
    let rerun = [(r, Some(proof))];
    let review = review::review("aaa", &rerun, &Actions::default(), &[], &[], None);
    assert_eq!(
        review.state,
        State::Failed,
        "a proof's own word is not enough"
    );
    let given = accept("aaa", &[gone("tests")]);
    let review = review::review("aaa", &rerun, &Actions::default(), &[], &[], Some(&given));
    assert_eq!(review.state, State::Approved);
    assert!(
        review.summary().contains("accepted by admin"),
        "{}",
        review.summary()
    );
    // An accept of other weakenings, or of another head, covers nothing here.
    let other = accept("aaa", &[gone("lint")]);
    let review = review::review("aaa", &rerun, &Actions::default(), &[], &[], Some(&other));
    assert_eq!(review.state, State::Failed);
    assert!(review.accept.is_none());
    let elsewhere = accept("bbb", &[gone("tests")]);
    assert_eq!(
        review::review(
            "aaa",
            &rerun,
            &Actions::default(),
            &[],
            &[],
            Some(&elsewhere)
        )
        .state,
        State::Failed
    );
}

#[test]
fn a_proof_holds_its_weakenings_unless_its_accept_covers_exactly_them_at_its_head() {
    let mut proof = Proof::new("aaa".into(), "aaa".into(), "main".into());
    proof.weakened = vec![gone("tests"), gone("lint")];
    assert_eq!(proof.blockers().len(), 2);
    proof.accepted = Some(accept("aaa", &[gone("lint"), gone("tests")]));
    assert!(proof.blockers().is_empty(), "in any order");
    proof.accepted = Some(accept("aaa", &[gone("tests")]));
    assert_eq!(proof.blockers().len(), 2, "fewer");
    proof.accepted = Some(accept("bbb", &[gone("tests"), gone("lint")]));
    assert_eq!(proof.blockers().len(), 2, "another head");
}

#[test]
fn a_failed_check_fails_the_review_even_without_its_reason() {
    let (r, proof) = run("r1", "aaa", RunState::Done, &[]);
    let mut proof = proof.unwrap();
    let mut failed = entry("tests", Ran::Passed);
    failed.outcome = Ran::Failed;
    proof.checks.push(failed);
    let review = review::review(
        "aaa",
        &[(r, Some(proof))],
        &Actions::default(),
        &[],
        &[],
        None,
    );
    assert_eq!(review.state, State::Failed);
}

#[test]
fn a_cancelled_run_ends_the_review_without_a_proof_and_a_newer_run_counts() {
    let cancelled = run("r1", "aaa", RunState::Cancelled, &[]);
    let review = review::review(
        "aaa",
        std::slice::from_ref(&cancelled),
        &Actions::default(),
        &[],
        &[],
        None,
    );
    assert_eq!(review.state, State::Cancelled);
    let again = run("r2", "aaa", RunState::Running, &[]);
    let review = review::review(
        "aaa",
        &[again, cancelled],
        &Actions::default(),
        &[],
        &[],
        None,
    );
    assert_eq!(review.state, State::Running);
}

#[test]
fn the_summary_lists_what_waits() {
    let runs = [run(
        "r1",
        "aaa1234",
        RunState::Done,
        &[still("card.png", Change::Changed)],
    )];
    let review = review::review("aaa1234", &runs, &Actions::default(), &[], &[], None);
    assert_eq!(review.title(), "1 picture waits for a look at aaa1234");
    assert_eq!(review.title.read, "1 picture waits for a look at aaa1234");
    assert_eq!(
        review.title.write,
        "1 picture waits for your look at aaa1234"
    );
    assert_eq!(
        review.title.admin,
        "1 picture waits for your look at aaa1234"
    );
    assert!(
        review.summary().contains("`screens` card.png"),
        "{}",
        review.summary()
    );
}

fn note(picture: &str, words: &str, state: NoteState) -> Note {
    Note {
        id: format!("n-{words}"),
        check: "screens".into(),
        picture: picture.into(),
        run: "r0".into(),
        head: "aaa".into(),
        side: Side::After,
        region: Region {
            x: 0.1,
            y: 0.6,
            w: 0.3,
            h: 0.2,
        },
        time: None,
        words: words.into(),
        by: "writer".into(),
        at: 4.0,
        state,
        resolved_by: None,
        link: None,
    }
}

#[test]
fn an_open_note_makes_its_picture_need_change_and_the_summary_lists_the_notes() {
    let runs = [run(
        "r1",
        "aaa1234",
        RunState::Done,
        &[
            still("card.png", Change::Changed),
            still("menu.png", Change::Changed),
        ],
    )];
    let notes = [
        note(
            "card.png",
            "The button text is *cut* off @someone",
            NoteState::Open,
        ),
        note("card.png", "an old one", NoteState::Resolved),
    ];
    let review = review::review("aaa1234", &runs, &Actions::default(), &[], &notes, None);
    assert_eq!(review.state, State::NeedsChange);
    assert_eq!(review.title(), "1 picture needs change at aaa1234");
    assert_eq!(review.notes.len(), 1, "only open notes wait");
    let waiting: Vec<&str> = review.waiting().map(|p| p.name.as_str()).collect();
    assert_eq!(
        waiting,
        ["menu.png"],
        "a picture with a note waits for no look"
    );
    let summary = review.summary();
    assert!(summary.contains("Pictures that need change"), "{summary}");
    assert!(summary.contains("`screens` card.png, after"), "{summary}");
    // A person's words never become markup or a mention.
    assert!(summary.contains(r"\*cut\*"), "{summary}");
    assert!(summary.contains(r"\@someone"), "{summary}");
    assert!(!summary.contains("an old one"), "{summary}");
}

#[test]
fn an_open_note_outweighs_looks_right_until_it_is_resolved() {
    let runs = [run(
        "r1",
        "aaa",
        RunState::Done,
        &[still("card.png", Change::Changed)],
    )];
    let right = verdict("card.png", &runs[0].0.checks[0].keys["card.png"], "aaa");
    let open = [note("card.png", "too tight", NoteState::Open)];
    let review = review::review(
        "aaa",
        &runs,
        &Actions::default(),
        std::slice::from_ref(&right),
        &open,
        None,
    );
    assert_eq!(review.state, State::NeedsChange);

    // Resolving the last note: the looks right verdict on its content holds again.
    let resolved = [note("card.png", "too tight", NoteState::Resolved)];
    let review = review::review("aaa", &runs, &Actions::default(), &[right], &resolved, None);
    assert_eq!(review.state, State::Approved);
    // Without one, the picture is back to not looked at yet.
    let review = review::review("aaa", &runs, &Actions::default(), &[], &resolved, None);
    assert_eq!(review.state, State::Waits);
    assert_eq!(review.waiting().count(), 1);
}

/// Which leg each run is, as the webhook named its job: `a*` the linux leg, `b*` the macos one.
fn two_legs() -> Actions {
    let legs = [
        ("a1", "proof (linux)"),
        ("a2", "proof (linux)"),
        ("b1", "proof (macos)"),
        ("b2", "proof (macos)"),
    ]
    .into_iter()
    .map(|(run, name)| (run.to_owned(), name.to_owned()))
    .collect();
    Actions {
        legs,
        ..Actions::default()
    }
}

#[test]
fn a_cancelled_rerun_of_a_leg_leaves_its_older_run_in_review() {
    let runs = [
        run("a2", "aaa", RunState::Cancelled, &[]),
        run("b1", "aaa", RunState::Done, &[]),
        run(
            "a1",
            "aaa",
            RunState::Done,
            &[still("card.png", Change::Changed)],
        ),
    ];
    let review = review::review("aaa", &runs, &two_legs(), &[], &[], None);
    assert_eq!(review.state, State::Waits, "{review:#?}");
    let waiting: Vec<&str> = review.waiting().map(|p| p.name.as_str()).collect();
    assert_eq!(waiting, ["card.png"]);

    let (a1, proof) = run("a1", "aaa", RunState::Done, &[]);
    let mut proof = proof.unwrap();
    proof.checks.push(entry("tests", Ran::Failed));
    let runs = [
        run("a2", "aaa", RunState::Cancelled, &[]),
        run("b1", "aaa", RunState::Done, &[]),
        (a1, Some(proof)),
    ];
    let review = review::review("aaa", &runs, &two_legs(), &[], &[], None);
    assert_eq!(review.state, State::Failed, "{review:#?}");
}

#[test]
fn a_leg_whose_every_run_ended_without_its_proof_keeps_the_review_from_approving() {
    let runs = [
        run("a1", "aaa", RunState::Cancelled, &[]),
        run("b1", "aaa", RunState::Done, &[]),
    ];
    let review = review::review("aaa", &runs, &two_legs(), &[], &[], None);
    assert_eq!(review.state, State::Cancelled, "{review:#?}");
    assert_eq!(
        review.title(),
        "A CI job of aaa ended without its proof: run it again"
    );
}

/// A run of a split proof: leg `leg` of a plan of `planned`.
fn leg_run(id: &str, state: RunState, leg: &str, planned: &[&str]) -> (Run, Option<Proof>) {
    let (mut r, proof) = run(id, "aaa", state, &[]);
    if let Source::Ci {
        leg: l, planned: p, ..
    } = &mut r.source
    {
        *l = Some(leg.into());
        *p = planned.iter().map(|s| (*s).into()).collect();
    }
    (r, proof)
}

#[test]
fn a_split_proof_waits_for_every_planned_leg() {
    let plan = ["linux", "mac"];
    let one = [leg_run("r1", RunState::Done, "linux", &plan)];
    assert_eq!(
        review::review("aaa", &one, &Actions::default(), &[], &[], None).state,
        State::Running
    );
    let both = [
        leg_run("r2", RunState::Done, "mac", &plan),
        leg_run("r1", RunState::Done, "linux", &plan),
    ];
    assert_eq!(
        review::review("aaa", &both, &Actions::default(), &[], &[], None).state,
        State::Approved
    );
}

/// A planned leg whose job never opened a run, once its workflow run attempt ended, is a leg
/// without its proof: the review stops waiting for it.
#[test]
fn a_planned_leg_that_never_opened_ends_with_its_attempt() {
    let plan = ["linux", "mac"];
    let one = [leg_run("r1", RunState::Done, "linux", &plan)];
    let ended = Actions {
        ended: BTreeSet::from([(1, 1)]),
        ..Actions::default()
    };
    assert_eq!(
        review::review("aaa", &one, &ended, &[], &[], None).state,
        State::Cancelled
    );
    // Another attempt ending leaves this one's legs to come.
    let other = Actions {
        ended: BTreeSet::from([(1, 2), (2, 1)]),
        ..Actions::default()
    };
    assert_eq!(
        review::review("aaa", &one, &other, &[], &[], None).state,
        State::Running
    );
    // A leg that failed fails the review, whether or not the other came.
    let (run, proof) = leg_run("r1", RunState::Done, "linux", &plan);
    let mut proof = proof.unwrap();
    proof.checks.push(entry("tests", Ran::Failed));
    let failed = [(run, Some(proof))];
    assert_eq!(
        review::review("aaa", &failed, &ended, &[], &[], None).state,
        State::Failed
    );
    // A run of the missing leg in the next attempt, as running the failed jobs again opens,
    // proves it.
    let (mut again, proof) = leg_run("r2", RunState::Done, "mac", &plan);
    if let Source::Ci { run_attempt, .. } = &mut again.source {
        *run_attempt = 2;
    }
    let both = [
        (again, proof),
        leg_run("r1", RunState::Done, "linux", &plan),
    ];
    assert_eq!(
        review::review("aaa", &both, &ended, &[], &[], None).state,
        State::Approved
    );
}

/// A leg run `id` of workflow run `run_id`, attempt `attempt`.
fn leg_run_of(
    id: &str,
    leg: &str,
    planned: &[&str],
    run_id: u64,
    attempt: u32,
) -> (Run, Option<Proof>) {
    let (mut run, proof) = leg_run(id, RunState::Done, leg, planned);
    if let Source::Ci {
        run_id: r,
        run_attempt: a,
        ..
    } = &mut run.source
    {
        *r = run_id;
        *a = attempt;
    }
    (run, proof)
}

/// A leg that never reached the host is named in the review, with the job to run again: the
/// one of its attempt whose name names the leg.
#[test]
fn a_missing_leg_is_named_with_its_job() {
    let one = [leg_run("r1", RunState::Done, "linux", &["linux", "macos"])];
    let actions = Actions {
        ended: BTreeSet::from([(1, 1)]),
        jobs: BTreeMap::from([(
            (1, 1),
            vec![
                "plan".to_owned(),
                "leg linux".to_owned(),
                "leg macos".to_owned(),
                "proof".to_owned(),
            ],
        )]),
        ..Actions::default()
    };
    let review = review::review("aaa", &one, &actions, &[], &[], None);
    assert_eq!(review.state, State::Cancelled);
    assert_eq!(
        review.missing,
        [Missing::Leg {
            leg: "macos".into(),
            job: Some("leg macos".into()),
            run_id: 1,
            run_attempt: 1,
        }]
    );
    assert_eq!(
        review.title(),
        "leg macos of aaa never reached the host: re-run job \"leg macos\""
    );
    assert!(
        review.summary().contains("never reached the host"),
        "{}",
        review.summary()
    );
    // A job GitHub has not named yet, or two that both name it, leave the job unknown.
    let unnamed = Actions {
        jobs: BTreeMap::new(),
        ..actions.clone()
    };
    let review = review::review("aaa", &one, &unnamed, &[], &[], None);
    assert_eq!(
        review.title(),
        "leg macos of aaa never reached the host: re-run its job"
    );
    // A run that opened and was cancelled keeps its own words.
    let cancelled = [leg_run("r1", RunState::Cancelled, "linux", &["linux"])];
    let review = review::review("aaa", &cancelled, &actions, &[], &[], None);
    assert_eq!(review.state, State::Cancelled);
    assert!(review.missing.is_empty(), "{review:#?}");
    assert_eq!(
        review.title(),
        "A CI job of aaa ended without its proof: run it again"
    );
}

/// A head no run of reached the host stays waiting for CI until the repository's proof
/// workflow ends for it; then nothing will come.
#[test]
fn a_proof_workflow_that_ended_without_a_run_cancels_the_review() {
    let waiting = review::review("aaa", &[], &Actions::default(), &[], &[], None);
    assert_eq!(waiting.state, State::Ci);
    // Another workflow of the head ending says nothing of the proof.
    let other = Actions {
        ended: BTreeSet::from([(7, 1)]),
        ..Actions::default()
    };
    let review = review::review("aaa", &[], &other, &[], &[], None);
    assert_eq!(review.state, State::Ci);
    let ended = Actions {
        ended: BTreeSet::from([(7, 1), (8, 1), (8, 2)]),
        proof_ended: BTreeSet::from([(8, 1), (8, 2)]),
        ..Actions::default()
    };
    let review = review::review("aaa", &[], &ended, &[], &[], None);
    assert_eq!(review.state, State::Cancelled);
    assert_eq!(
        review.missing,
        [Missing::Workflow {
            run_id: 8,
            run_attempt: 2
        }]
    );
    assert_eq!(
        review.title(),
        "The proof workflow of aaa ended without reaching the host: re-run it"
    );
    // A run of the head that reached the host, in any attempt, is judged instead.
    let runs = [run("r1", "aaa", RunState::Done, &[])];
    let review = review::review("aaa", &runs, &ended, &[], &[], None);
    assert_eq!(review.state, State::Approved);
    assert!(review.missing.is_empty(), "{review:#?}");
}

/// The newest attempt of a workflow run plans its legs: running every job again plans anew,
/// and may leave out a leg whose pass it reuses from the base commit.
#[test]
fn a_newer_attempt_plans_the_legs_of_its_workflow_run() {
    let first_ended = Actions {
        ended: BTreeSet::from([(1, 1)]),
        ..Actions::default()
    };
    // Attempt 1 planned linux and mac, mac never opened, and attempt 1 completed. Attempt 2
    // planned linux alone, and it passed.
    let runs = [
        leg_run_of("r2", "linux", &["linux"], 1, 2),
        leg_run_of("r1", "linux", &["linux", "mac"], 1, 1),
    ];
    assert_eq!(
        review::review("aaa", &runs, &first_ended, &[], &[], None).state,
        State::Approved
    );
    // Attempt 2 planned both again: its linux run opened, its mac run is still to come.
    let runs = [
        leg_run_of("r2", "linux", &["linux", "mac"], 1, 2),
        leg_run_of("r1", "linux", &["linux", "mac"], 1, 1),
    ];
    assert_eq!(
        review::review("aaa", &runs, &first_ended, &[], &[], None).state,
        State::Running
    );
}

/// Two workflow runs of the head each plan legs: a leg one of them still runs is to come, even
/// when the other ended without it.
#[test]
fn a_leg_waits_while_any_workflow_run_that_planned_it_runs() {
    let runs = [
        leg_run_of("r2", "linux", &["linux", "mac"], 2, 1),
        leg_run_of("r1", "linux", &["linux", "mac"], 1, 1),
    ];
    let one_ended = Actions {
        ended: BTreeSet::from([(1, 1)]),
        ..Actions::default()
    };
    assert_eq!(
        review::review("aaa", &runs, &one_ended, &[], &[], None).state,
        State::Running
    );
    let both_ended = Actions {
        ended: BTreeSet::from([(1, 1), (2, 1)]),
        ..Actions::default()
    };
    assert_eq!(
        review::review("aaa", &runs, &both_ended, &[], &[], None).state,
        State::Cancelled
    );
}

#[test]
fn a_leg_the_run_names_replaces_its_older_run_without_a_job_name() {
    // The newer run of leg linux failed; GitHub has named neither job yet.
    let (newer, proof) = leg_run("r2", RunState::Done, "linux", &["linux"]);
    let mut proof = proof.unwrap();
    proof.checks.push(entry("tests", Ran::Failed));
    let runs = [
        (newer, Some(proof)),
        leg_run("r1", RunState::Done, "linux", &["linux"]),
    ];
    assert_eq!(
        review::review("aaa", &runs, &Actions::default(), &[], &[], None).state,
        State::Failed
    );
    let (newer, proof) = leg_run("r3", RunState::Done, "linux", &["linux"]);
    let (older, mut failed) = leg_run("r1", RunState::Done, "linux", &["linux"]);
    failed
        .as_mut()
        .unwrap()
        .checks
        .push(entry("tests", Ran::Failed));
    let runs = [(newer, proof), (older, failed)];
    assert_eq!(
        review::review("aaa", &runs, &Actions::default(), &[], &[], None).state,
        State::Approved
    );
}

#[test]
fn the_review_names_the_preview_its_proof_deployed() {
    let (r, proof) = run("r1", "aaa", RunState::Done, &[]);
    let mut proof = proof.unwrap();
    proof.preview = Some("https://feat-site.example.dev/".into());
    let review = review::review(
        "aaa",
        &[(r, Some(proof))],
        &Actions::default(),
        &[],
        &[],
        None,
    );
    assert_eq!(
        review.preview.as_deref(),
        Some("https://feat-site.example.dev/")
    );
    let none = review::review(
        "aaa",
        &[run("r1", "aaa", RunState::Done, &[])],
        &Actions::default(),
        &[],
        &[],
        None,
    );
    assert_eq!(none.preview, None);
}

#[test]
fn a_preview_that_is_no_plain_web_address_is_left_out() {
    for bad in [
        "javascript:alert(1)",
        "data:text/html,hi",
        "https://x.example.dev/\n@team",
        "https://x.example.dev/ [click](https://evil.example)",
        "https://",
    ] {
        let (r, proof) = run("r1", "aaa", RunState::Done, &[]);
        let mut proof = proof.unwrap();
        proof.preview = Some(bad.into());
        let review = review::review(
            "aaa",
            &[(r, Some(proof))],
            &Actions::default(),
            &[],
            &[],
            None,
        );
        assert_eq!(review.preview, None, "{bad}");
    }
}

#[test]
fn a_picture_that_changed_since_it_looked_right_waits_and_says_so() {
    let before = run(
        "r0",
        "aaa",
        RunState::Done,
        &[still("card.png", Change::Changed)],
    );
    let given = verdict("card.png", &before.0.checks[0].keys["card.png"], "aaa");
    let now = run(
        "r1",
        "bbb",
        RunState::Done,
        &[still("card.png", Change::Changed)],
    );
    let review = review::review(
        "bbb",
        &[now, before],
        &Actions::default(),
        std::slice::from_ref(&given),
        &[],
        None,
    );
    assert_eq!(
        review.state,
        State::Waits,
        "other content waits for a look again"
    );
    let since = review.pictures[0]
        .changed_since
        .as_ref()
        .expect("it says it changed");
    assert_eq!(
        since.verdict.as_ref(),
        Some(&given),
        "since the looks right on its old content"
    );
    assert!(since.notes.is_empty(), "no note: {since:?}");
    assert!(
        review
            .summary()
            .contains("- `screens` card.png, changed since writer said it looks right"),
        "{}",
        review.summary()
    );
}

#[test]
fn a_picture_that_looks_right_now_has_not_changed_since() {
    let now = run(
        "r1",
        "bbb",
        RunState::Done,
        &[still("card.png", Change::Changed)],
    );
    let old = verdict("card.png", "still:old:old", "aaa");
    let given = verdict("card.png", &now.0.checks[0].keys["card.png"], "bbb");
    let review = review::review("bbb", &[now], &Actions::default(), &[old, given], &[], None);
    assert_eq!(review.state, State::Approved);
    assert_eq!(
        review.pictures[0].changed_since, None,
        "its content has a verdict"
    );
}

#[test]
fn an_open_note_on_older_content_says_the_picture_changed_since() {
    let before = run(
        "r0",
        "aaa",
        RunState::Done,
        &[still("card.png", Change::Changed)],
    );
    let now = run(
        "r1",
        "bbb",
        RunState::Done,
        &[still("card.png", Change::Changed)],
    );
    let older = note("card.png", "made on r0", NoteState::Open);
    let current = Note {
        run: "r1".into(),
        head: "bbb".into(),
        ..note("card.png", "made on r1", NoteState::Open)
    };
    let review = review::review(
        "bbb",
        &[now, before],
        &Actions::default(),
        &[],
        &[older.clone(), current],
        None,
    );
    assert_eq!(
        review.state,
        State::NeedsChange,
        "a note holds until resolved"
    );
    let since = review.pictures[0]
        .changed_since
        .as_ref()
        .expect("it says it changed");
    assert_eq!(
        since.notes,
        vec![older.id.clone()],
        "only the note on other content"
    );
    assert!(
        review
            .summary()
            .contains("- `screens` card.png, changed since its note"),
        "{}",
        review.summary()
    );
}

#[test]
fn a_note_whose_run_is_not_known_says_nothing_about_change() {
    let now = run(
        "r1",
        "bbb",
        RunState::Done,
        &[still("card.png", Change::Changed)],
    );
    let review = review::review(
        "bbb",
        &[now],
        &Actions::default(),
        &[],
        &[note("card.png", "made on r0", NoteState::Open)],
        None,
    );
    assert_eq!(
        review.pictures[0].changed_since, None,
        "nothing known to compare with"
    );
}

/// Run `r` with its check's comparison of picture `name` now with the picture of key `then`.
fn found(mut r: (Run, Option<Proof>), name: &str, then: &str, same: bool) -> (Run, Option<Proof>) {
    r.0.checks[0]
        .since
        .entry(name.into())
        .or_default()
        .insert(then.into(), same);
    r
}

#[test]
fn a_pictures_key_is_its_own_file_so_another_base_keeps_it() {
    let mut other_base = still("card.png", Change::Changed);
    other_base.before = Some("looks/s/before/other.png".into());
    let sha = |path: &str| Some(format!("sha of {path}"));
    assert_eq!(
        review::key(&still("card.png", Change::Changed), sha),
        review::key(&other_base, sha)
    );
    assert_eq!(
        review::key(&still("card.png", Change::Changed), sha).as_deref(),
        Some("sha of looks/s/after/card.png")
    );
    assert_eq!(
        review::key(&still("card.png", Change::Gone), sha).as_deref(),
        Some("gone:sha of looks/s/before/card.png"),
        "a gone picture is the one it was"
    );
}

#[test]
fn a_picture_its_run_found_the_same_as_the_one_that_looked_right_keeps_its_verdict() {
    let before = run(
        "r0",
        "aaa",
        RunState::Done,
        &[still("card.png", Change::Changed)],
    );
    let then = before.0.checks[0].keys["card.png"].clone();
    let given = verdict("card.png", &then, "aaa");
    let now = found(
        run(
            "r1",
            "bbb",
            RunState::Done,
            &[still("card.png", Change::Changed)],
        ),
        "card.png",
        &then,
        true,
    );
    let review = review::review(
        "bbb",
        &[now, before],
        &Actions::default(),
        std::slice::from_ref(&given),
        &[],
        None,
    );
    assert_eq!(review.state, State::Approved, "{:?}", review.pictures);
    assert_eq!(review.pictures[0].verdict.as_ref(), Some(&given));
    assert_eq!(review.pictures[0].changed_since, None);
}

#[test]
fn a_picture_its_run_found_changed_since_it_looked_right_waits() {
    let before = run(
        "r0",
        "aaa",
        RunState::Done,
        &[still("card.png", Change::Changed)],
    );
    let then = before.0.checks[0].keys["card.png"].clone();
    let given = verdict("card.png", &then, "aaa");
    let now = found(
        run(
            "r1",
            "bbb",
            RunState::Done,
            &[still("card.png", Change::Changed)],
        ),
        "card.png",
        &then,
        false,
    );
    let review = review::review(
        "bbb",
        &[now, before],
        &Actions::default(),
        std::slice::from_ref(&given),
        &[],
        None,
    );
    assert_eq!(review.state, State::Waits);
    assert_eq!(
        review.pictures[0]
            .changed_since
            .as_ref()
            .and_then(|c| c.verdict.as_ref()),
        Some(&given)
    );
}

#[test]
fn an_open_note_on_a_picture_its_run_found_the_same_is_no_change_since() {
    let before = run(
        "r0",
        "aaa",
        RunState::Done,
        &[still("card.png", Change::Changed)],
    );
    let then = before.0.checks[0].keys["card.png"].clone();
    let now = found(
        run(
            "r1",
            "bbb",
            RunState::Done,
            &[still("card.png", Change::Changed)],
        ),
        "card.png",
        &then,
        true,
    );
    let review = review::review(
        "bbb",
        &[now, before],
        &Actions::default(),
        &[],
        &[note("card.png", "made on r0", NoteState::Open)],
        None,
    );
    assert_eq!(review.state, State::NeedsChange);
    assert_eq!(review.pictures[0].changed_since, None);
}

#[test]
fn a_run_compares_each_picture_with_the_ones_a_person_said_something_about() {
    let r0 = run(
        "r0",
        "aaa",
        RunState::Done,
        &[still("card.png", Change::Changed)],
    );
    let r1 = run(
        "r1",
        "bbb",
        RunState::Done,
        &[still("card.png", Change::Changed)],
    );
    let (k0, k1) = (
        r0.0.checks[0].keys["card.png"].clone(),
        r1.0.checks[0].keys["card.png"].clone(),
    );
    let older = Verdict {
        at: 1.0,
        ..verdict("card.png", "an older look", "aaa")
    };
    let newest = Verdict {
        at: 9.0,
        ..verdict("card.png", &k1, "bbb")
    };
    let open = note("card.png", "made on r0", NoteState::Open);
    let resolved = Note {
        run: "r1".into(),
        ..note("card.png", "resolved", NoteState::Resolved)
    };
    let approved = review::approved(&[r1, r0], &[older, newest], &[open, resolved]);
    let card: Vec<&str> = approved["screens"]["card.png"]
        .keys()
        .map(String::as_str)
        .collect();
    let mut want = vec![k0.as_str(), k1.as_str()];
    want.sort_unstable();
    assert_eq!(
        card, want,
        "the newest looks right and the open note's picture, not older looks or resolved notes"
    );
}

#[test]
fn an_approved_film_carries_the_marks_its_run_said() {
    // A run compares its film with the approved one timed from both films' marks, so the
    // approved key comes with the marks of the run the person looked at.
    let marks = Marks {
        marks: vec![Mark {
            name: "loaded".into(),
            at: 0.4,
            hidden: false,
        }],
        spans: Vec::new(),
    };
    let r0 = run(
        "r0",
        "aaa1234",
        RunState::Done,
        &[Item {
            marks: Some(marks.clone()),
            ..film(&[1])
        }],
    );
    let key = r0.0.checks[0].keys["checkout.webm"].clone();
    let approved = review::approved(&[r0], &[verdict("checkout.webm", &key, "aaa1234")], &[]);
    assert_eq!(approved["screens"]["checkout.webm"][&key], Some(marks));
}

#[test]
fn an_open_note_on_a_picture_the_head_does_not_show_lapses() {
    // Its picture was renamed or removed: nothing on the page shows it, so it holds nothing.
    let runs = [run(
        "r1",
        "aaa1234",
        RunState::Done,
        &[still("card/light.png", Change::Changed)],
    )];
    let notes = [note("card.png", "the old name", NoteState::Open)];
    let key = runs[0].0.checks[0].keys["card/light.png"].clone();
    let verdicts = [verdict("card/light.png", &key, "aaa1234")];
    let review = review::review(
        "aaa1234",
        &runs,
        &Actions::default(),
        &verdicts,
        &notes,
        None,
    );
    assert_eq!(review.state, State::Approved, "{review:#?}");
    assert!(review.notes.is_empty(), "{review:#?}");
}
