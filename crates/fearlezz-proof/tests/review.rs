//! Where review of a head stands: which pictures wait for a verdict, whether its weakenings
//! wait for an admin's accept, and what a verdict follows from run to run.

use std::collections::BTreeMap;

use fearlezz_proof::events::{Run, RunCheck, RunState, Source};
use fearlezz_proof::floor::{Part, Weakening};
use fearlezz_proof::look::{Change, Item, Look};
use fearlezz_proof::media::{Comparison, Pair, Verdict as Likeness};
use fearlezz_proof::notes::{Note, NoteState, Region, Side};
use fearlezz_proof::review::{self, Accept, State, Verdict};
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
        diff: None,
        webp: None,
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
    let review = review::review("aaa", &[], &BTreeMap::new(), &[], &[], None);
    assert_eq!(review.state, State::Ci);
}

#[test]
fn a_running_run_of_the_head_is_running_and_another_heads_runs_do_not_count() {
    let runs = [
        run("r2", "bbb", RunState::Running, &[]),
        run("r1", "aaa", RunState::Done, &[]),
    ];
    assert_eq!(
        review::review("bbb", &runs, &BTreeMap::new(), &[], &[], None).state,
        State::Running
    );
    assert_eq!(
        review::review("aaa", &runs, &BTreeMap::new(), &[], &[], None).state,
        State::Approved
    );
}

#[test]
fn a_failed_proof_fails_the_review() {
    let (r, proof) = run("r1", "aaa", RunState::Done, &[]);
    let mut proof = proof.unwrap();
    proof.checks.push(entry("tests", Ran::Failed));
    let review = review::review("aaa", &[(r, Some(proof))], &BTreeMap::new(), &[], &[], None);
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
    let review = review::review("aaa", &runs, &BTreeMap::new(), &[], &[], None);
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
        &BTreeMap::new(),
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
    let review = review::review("aaa", &runs, &BTreeMap::new(), &[], &[], None);
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
        &BTreeMap::new(),
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
    let review = review::review("ccc", &[other], &BTreeMap::new(), &[given], &[], None);
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
    let review = review::review("aaa", &runs, &BTreeMap::new(), &[], &[], None);
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
    let review = review::review("aaa", &rerun, &BTreeMap::new(), &[], &[], None);
    assert_eq!(
        review.state,
        State::Failed,
        "a proof's own word is not enough"
    );
    let given = accept("aaa", &[gone("tests")]);
    let review = review::review("aaa", &rerun, &BTreeMap::new(), &[], &[], Some(&given));
    assert_eq!(review.state, State::Approved);
    assert!(
        review.summary().contains("accepted by admin"),
        "{}",
        review.summary()
    );
    // An accept of other weakenings, or of another head, covers nothing here.
    let other = accept("aaa", &[gone("lint")]);
    let review = review::review("aaa", &rerun, &BTreeMap::new(), &[], &[], Some(&other));
    assert_eq!(review.state, State::Failed);
    assert!(review.accept.is_none());
    let elsewhere = accept("bbb", &[gone("tests")]);
    assert_eq!(
        review::review("aaa", &rerun, &BTreeMap::new(), &[], &[], Some(&elsewhere)).state,
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
    let review = review::review("aaa", &[(r, Some(proof))], &BTreeMap::new(), &[], &[], None);
    assert_eq!(review.state, State::Failed);
}

#[test]
fn a_cancelled_run_ends_the_review_without_a_proof_and_a_newer_run_counts() {
    let cancelled = run("r1", "aaa", RunState::Cancelled, &[]);
    let review = review::review(
        "aaa",
        std::slice::from_ref(&cancelled),
        &BTreeMap::new(),
        &[],
        &[],
        None,
    );
    assert_eq!(review.state, State::Cancelled);
    let again = run("r2", "aaa", RunState::Running, &[]);
    let review = review::review("aaa", &[again, cancelled], &BTreeMap::new(), &[], &[], None);
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
    let review = review::review("aaa1234", &runs, &BTreeMap::new(), &[], &[], None);
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
    let review = review::review("aaa1234", &runs, &BTreeMap::new(), &[], &notes, None);
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
        &BTreeMap::new(),
        std::slice::from_ref(&right),
        &open,
        None,
    );
    assert_eq!(review.state, State::NeedsChange);

    // Resolving the last note: the looks right verdict on its content holds again.
    let resolved = [note("card.png", "too tight", NoteState::Resolved)];
    let review = review::review("aaa", &runs, &BTreeMap::new(), &[right], &resolved, None);
    assert_eq!(review.state, State::Approved);
    // Without one, the picture is back to not looked at yet.
    let review = review::review("aaa", &runs, &BTreeMap::new(), &[], &resolved, None);
    assert_eq!(review.state, State::Waits);
    assert_eq!(review.waiting().count(), 1);
}

/// Which leg each run is, as the webhook named its job: `a*` the linux leg, `b*` the macos one.
fn two_legs() -> BTreeMap<String, String> {
    [
        ("a1", "proof (linux)"),
        ("a2", "proof (linux)"),
        ("b1", "proof (macos)"),
        ("b2", "proof (macos)"),
    ]
    .into_iter()
    .map(|(run, name)| (run.to_owned(), name.to_owned()))
    .collect()
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
        review::review("aaa", &one, &BTreeMap::new(), &[], &[], None).state,
        State::Running
    );
    let both = [
        leg_run("r2", RunState::Done, "mac", &plan),
        leg_run("r1", RunState::Done, "linux", &plan),
    ];
    assert_eq!(
        review::review("aaa", &both, &BTreeMap::new(), &[], &[], None).state,
        State::Approved
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
        review::review("aaa", &runs, &BTreeMap::new(), &[], &[], None).state,
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
        review::review("aaa", &runs, &BTreeMap::new(), &[], &[], None).state,
        State::Approved
    );
}

#[test]
fn the_review_names_the_preview_its_proof_deployed() {
    let (r, proof) = run("r1", "aaa", RunState::Done, &[]);
    let mut proof = proof.unwrap();
    proof.preview = Some("https://feat-site.example.dev/".into());
    let review = review::review("aaa", &[(r, Some(proof))], &BTreeMap::new(), &[], &[], None);
    assert_eq!(
        review.preview.as_deref(),
        Some("https://feat-site.example.dev/")
    );
    let none = review::review(
        "aaa",
        &[run("r1", "aaa", RunState::Done, &[])],
        &BTreeMap::new(),
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
        let review = review::review("aaa", &[(r, Some(proof))], &BTreeMap::new(), &[], &[], None);
        assert_eq!(review.preview, None, "{bad}");
    }
}
