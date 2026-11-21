//! The page's TypeScript types (`ee/app/src/proof.ts`) are generated from the proof model in
//! `crates/fearlezz-proof`, so the page and the host read one shape. This fails when the file
//! drifts from the model; `FEARLEZZ_WRITE_TYPES=1 cargo test --manifest-path ee/Cargo.toml
//! --test types` writes it again.

use std::path::Path;

use fearlezz_proof::events::{
    Change, Latest, Listed, Listing, Live, Run, RunCheck, RunState, Share, Snapshot, Source,
    Standing as ListedStanding, Work, WorkChange,
};
use fearlezz_proof::findings::{Finding, Level};
use fearlezz_proof::floor::Weakening;
use fearlezz_proof::look::{Change as PictureChange, DiffLine, Item as Picture, Look};
use fearlezz_proof::media::{Comparison, Moment, Pair, Verdict as Likeness};
use fearlezz_proof::notes::{Edit as NoteEdit, Note, NoteState, Region, Side as NoteSide};
use fearlezz_proof::review::{
    Accept, ChangedSince, Job, Picture as ReviewPicture, Review, Role, Standing as PictureStanding,
    State as ReviewState, Title as ReviewTitle, Verdict as PictureVerdict, Viewer,
};
use fearlezz_proof::{Case, Check, Item, Pull, Ran, Red, Verdict};
use ts_rs::{Config, TS};

/// Every type the page reads, each declared once, those it uses after it.
fn declarations() -> String {
    // Every count and sequence number fits a JavaScript number by far.
    let cfg = Config::new().with_large_int("number");
    let decls = [
        Live::decl(&cfg),
        Listing::decl(&cfg),
        Listed::decl(&cfg),
        ListedStanding::decl(&cfg),
        Latest::decl(&cfg),
        Snapshot::decl(&cfg),
        Change::decl(&cfg),
        WorkChange::decl(&cfg),
        Review::decl(&cfg),
        ReviewState::decl(&cfg),
        ReviewTitle::decl(&cfg),
        ReviewPicture::decl(&cfg),
        ChangedSince::decl(&cfg),
        PictureStanding::decl(&cfg),
        PictureVerdict::decl(&cfg),
        Accept::decl(&cfg),
        Note::decl(&cfg),
        NoteEdit::decl(&cfg),
        NoteSide::decl(&cfg),
        Region::decl(&cfg),
        NoteState::decl(&cfg),
        Weakening::decl(&cfg),
        Job::decl(&cfg),
        Viewer::decl(&cfg),
        Role::decl(&cfg),
        Share::decl(&cfg),
        Work::decl(&cfg),
        Pull::decl(&cfg),
        Run::decl(&cfg),
        RunState::decl(&cfg),
        Source::decl(&cfg),
        RunCheck::decl(&cfg),
        Look::decl(&cfg),
        Picture::decl(&cfg),
        PictureChange::decl(&cfg),
        DiffLine::decl(&cfg),
        Comparison::decl(&cfg),
        Likeness::decl(&cfg),
        Moment::decl(&cfg),
        Pair::decl(&cfg),
        Check::decl(&cfg),
        Ran::decl(&cfg),
        Case::decl(&cfg),
        Finding::decl(&cfg),
        Level::decl(&cfg),
        Item::decl(&cfg),
        Red::decl(&cfg),
        Verdict::decl(&cfg),
    ];
    let mut text = String::from(
        "// Generated from crates/fearlezz-proof by ee/host-tests/tests/types.rs; do not edit.\n\
         // FEARLEZZ_WRITE_TYPES=1 cargo test --manifest-path ee/Cargo.toml --test types\n",
    );
    for decl in decls {
        text.push_str("\nexport ");
        text.push_str(&decl);
        text.push('\n');
    }
    text
}

#[test]
fn the_pages_types_are_the_proof_model() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../app/src/proof.ts");
    let wanted = declarations();
    if std::env::var_os("FEARLEZZ_WRITE_TYPES").is_some() {
        std::fs::write(&path, &wanted).unwrap();
        return;
    }
    let found = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        found == wanted,
        "{} drifted from the proof model; write it again with FEARLEZZ_WRITE_TYPES=1 cargo test \
         --manifest-path ee/Cargo.toml --test types",
        path.display()
    );
}
