//! Where review of a head stands: which pictures wait for a verdict, whether its weakenings
//! wait for an admin's accept, and what a verdict follows from run to run.

use std::collections::BTreeMap;

use fearlezz_proof::events::{Run, RunCheck, RunState, Source};
use fearlezz_proof::floor::{Part, Weakening};
use fearlezz_proof::look::{Change, Item, Look};
use fearlezz_proof::media::{Comparison, Pair, Verdict as Likeness};
use fearlezz_proof::notes::{Note, NoteState, Region, Side};
use fearlezz_proof::review::{self, Accept, State, Verdict, VerdictKind};
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
            states_before: 3,
            states_after: 3,
            moments: Vec::new(),
            pairs: (0..3)
                .map(|s| pair(s, if changed.contains(&s) { 100 } else { 0 }))
                .collect(),
        }),
        flaky: false,
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
        look: Some(look.clone()),
    };
    let run = Run {
        id: id.into(),
        head: head.into(),
        base: "main".into(),
        source: Source::Ci {
            job: "proof".into(),
            run_url: None,
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
        kind: VerdictKind::LooksRight,
        by: "writer".into(),
        at: 5.0,
        run: "r0".into(),
        head: head.into(),
    }
}

#[test]
fn a_head_without_a_run_waits_for_ci() {
    let review = review::review("aaa", &[], &[], &[], None);
    assert_eq!(review.state, State::Ci);
}

#[test]
fn a_running_run_of_the_head_is_running_and_another_heads_runs_do_not_count() {
    let runs = [
        run("r2", "bbb", RunState::Running, &[]),
        run("r1", "aaa", RunState::Done, &[]),
    ];
    assert_eq!(
        review::review("bbb", &runs, &[], &[], None).state,
        State::Running
    );
    assert_eq!(
        review::review("aaa", &runs, &[], &[], None).state,
        State::Approved
    );
}

#[test]
fn a_failed_proof_fails_the_review() {
    let (r, proof) = run("r1", "aaa", RunState::Done, &[]);
    let mut proof = proof.unwrap();
    proof.checks.push(entry("tests", Ran::Failed));
    let review = review::review("aaa", &[(r, Some(proof))], &[], &[], None);
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
    let review = review::review("aaa", &runs, &[], &[], None);
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
    let review = review::review("aaa", &runs, &[verdict("card.png", &key, "aaa")], &[], None);
    assert_eq!(review.state, State::Approved);
    assert_eq!(review.pictures[0].verdict.as_ref().unwrap().by, "writer");
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
    let review = review::review("bbb", &[same], std::slice::from_ref(&given), &[], None);
    assert_eq!(review.state, State::Approved);
    assert_eq!(review.pictures[0].verdict.as_ref().unwrap().head, "aaa");

    let other = run(
        "r2",
        "ccc",
        RunState::Done,
        &[still("card.png", Change::Changed)],
    );
    let review = review::review("ccc", &[other], &[given], &[], None);
    assert_eq!(review.state, State::Waits);
}

#[test]
fn a_still_is_keyed_by_its_before_and_after_bytes() {
    let item = still("card.png", Change::Changed);
    let hash = |path: &str| Some(if path.contains("before") { "b1" } else { "a1" }.to_owned());
    assert_eq!(review::key(&item, hash).unwrap(), "still:b1:a1");
    let new = still("new.png", Change::New);
    assert_eq!(
        review::key(&new, |_| Some("a2".into())).unwrap(),
        "still:-:a2"
    );
    // A file whose bytes are not known yet has no key.
    assert_eq!(review::key(&item, |_| None), None);
}

#[test]
fn a_film_is_keyed_by_which_states_changed_not_by_its_bytes() {
    let a = review::key(&film(&[1, 2]), |_| Some("x".into())).unwrap();
    let b = review::key(&film(&[1, 2]), |_| Some("y".into())).unwrap();
    let c = review::key(&film(&[1]), |_| Some("x".into())).unwrap();
    assert_eq!(a, b, "another recording of the same change");
    assert_ne!(a, c, "another change");
}

#[test]
fn a_new_or_gone_film_is_keyed_by_its_bytes() {
    // Nothing compared it, so another recording is another film: its verdict never carries over.
    for change in [Change::New, Change::Gone] {
        let mut item = film(&[]);
        item.change = change;
        item.comparison = None;
        match change {
            Change::New => item.before = None,
            _ => item.after = None,
        }
        let a = review::key(&item, |_| Some("x".into())).unwrap();
        let b = review::key(&item, |_| Some("y".into())).unwrap();
        assert_ne!(a, b, "{change:?}: another recording");
        assert_eq!(review::key(&item, |_| None), None, "{change:?}: bytes not known yet");
    }
}

#[test]
fn weakenings_wait_for_an_admins_accept_of_the_head() {
    let (r, proof) = run("r1", "aaa", RunState::Done, &[]);
    let mut proof = proof.unwrap();
    proof.weakened = vec![Weakening {
        part: Part::Check("tests".into()),
        how: "it runs another command".into(),
    }];
    proof.weakenings_wait = true;
    let runs = [(r, Some(proof))];
    let review = review::review("aaa", &runs, &[], &[], None);
    assert_eq!(review.state, State::Waits);
    assert_eq!(review.weakened.len(), 1);
    let accept = Accept {
        head: "aaa".into(),
        by: "admin".into(),
        at: 9.0,
    };
    let review = review::review("aaa", &runs, &[], &[], Some(&accept));
    assert_eq!(review.state, State::Approved);
    assert!(
        review.summary().contains("accepted by admin"),
        "{}",
        review.summary()
    );
}

#[test]
fn a_failed_check_fails_the_review_even_without_its_reason() {
    let (r, proof) = run("r1", "aaa", RunState::Done, &[]);
    let mut proof = proof.unwrap();
    let mut failed = entry("tests", Ran::Passed);
    failed.outcome = Ran::Failed;
    proof.checks.push(failed);
    let review = review::review("aaa", &[(r, Some(proof))], &[], &[], None);
    assert_eq!(review.state, State::Failed);
}

#[test]
fn a_weakening_where_nothing_waits_fails_the_review() {
    let (r, proof) = run("r1", "aaa", RunState::Done, &[]);
    let mut proof = proof.unwrap();
    proof.weakened = vec![Weakening {
        part: Part::Check("tests".into()),
        how: "it is gone".into(),
    }];
    let review = review::review("aaa", &[(r, Some(proof))], &[], &[], None);
    assert_eq!(review.state, State::Failed);
}

#[test]
fn a_cancelled_run_ends_the_review_without_a_proof_and_a_newer_run_counts() {
    let cancelled = run("r1", "aaa", RunState::Cancelled, &[]);
    let review = review::review("aaa", std::slice::from_ref(&cancelled), &[], &[], None);
    assert_eq!(review.state, State::Cancelled);
    let again = run("r2", "aaa", RunState::Running, &[]);
    let review = review::review("aaa", &[again, cancelled], &[], &[], None);
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
    let review = review::review("aaa1234", &runs, &[], &[], None);
    assert_eq!(review.title(), "1 picture waits for a look at aaa1234");
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
    }
}

#[test]
fn a_picture_that_needs_change_fails_the_review_and_the_summary_lists_the_notes() {
    let runs = [run(
        "r1",
        "aaa1234",
        RunState::Done,
        &[
            still("card.png", Change::Changed),
            still("menu.png", Change::Changed),
        ],
    )];
    let key = runs[0].0.checks[0].keys["card.png"].clone();
    let mut said = verdict("card.png", &key, "aaa1234");
    said.kind = VerdictKind::NeedsChange;
    let notes = [
        note(
            "card.png",
            "The button text is *cut* off @someone",
            NoteState::Open,
        ),
        note("card.png", "an old one", NoteState::Resolved),
    ];
    let review = review::review("aaa1234", &runs, &[said], &notes, None);
    assert_eq!(review.state, State::NeedsChange);
    assert_eq!(review.title(), "1 picture needs change at aaa1234");
    assert_eq!(review.notes.len(), 1, "only open notes wait");
    let summary = review.summary();
    assert!(summary.contains("Pictures that need change"), "{summary}");
    assert!(summary.contains("`screens` card.png, after"), "{summary}");
    // A person's words never become markup or a mention.
    assert!(summary.contains(r"\*cut\*"), "{summary}");
    assert!(summary.contains(r"\@someone"), "{summary}");
    assert!(!summary.contains("an old one"), "{summary}");
}

#[test]
fn a_needs_change_verdict_holds_only_for_the_content_it_was_given_on() {
    // The agent changed the picture: its new content waits for a fresh verdict, while the
    // open notes on it stay.
    let first = run(
        "r1",
        "aaa",
        RunState::Done,
        &[still("card.png", Change::Changed)],
    );
    let mut said = verdict("card.png", &first.0.checks[0].keys["card.png"], "aaa");
    said.kind = VerdictKind::NeedsChange;
    let notes = [note("card.png", "too tight", NoteState::Open)];
    let later = run(
        "r2",
        "bbb",
        RunState::Done,
        &[still("card.png", Change::Changed)],
    );
    let review = review::review("bbb", &[later], &[said], &notes, None);
    assert_eq!(review.state, State::Waits);
    assert!(review.pictures[0].verdict.is_none());
    assert_eq!(review.notes[0].words, "too tight");
}
