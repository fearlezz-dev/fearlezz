// A run's pictures: counts that filter, grouping by the parts of their names, every picture in
// the order that directs attention, the one on the stage, the notes on it and the verdict. j
// and k step between pictures, 1 to 5 pick the way to compare.

import { For, Match, Show, Switch, createEffect, createMemo, createSignal, on, onCleanup, onMount } from "solid-js";
import { fileUrl, grantOf } from "./grant";
import {
  GROUP_WORDS,
  type Group,
  type GroupBy,
  type Shown,
  WORDS,
  counts,
  grouped,
  groupings,
  isFilm,
  isText,
  picturesOf,
  title,
} from "./pictures";
import { type Draft, type Shown as Noted, fromWhere, makeNote, notesOn, resolveNote, sidesOf } from "./notes";
import type { PictureChange, Run, Snapshot } from "./proof";
import { type Viewer, canSay, carried, localBy, maySignIn, onlyViews, signIn, standingOf, toLookAt, verdict as say, verdictOf } from "./review";
import { MODES, type Mode, type Noting, Stage } from "./Stage";
import { ago } from "./ago";

// The way to compare and the grouping stay as a person moves between pictures and runs.
const [mode, setMode] = createSignal<Mode>("side");
const [by, setBy] = createSignal<GroupBy>("change");

export function PicturesPanel(props: { run: Run; state: Snapshot; viewer: Viewer | undefined }) {
  const files = grantOf(() => props.run.id);
  const pictures = createMemo(() => picturesOf(props.run));
  const [filter, setFilter] = createSignal<PictureChange | null>(null);
  const [unfolded, setUnfolded] = createSignal<ReadonlySet<string>>(new Set());
  const [picked, setPicked] = createSignal<string | null>(null);
  const options = createMemo(() => groupings(pictures()));
  const grouping = () => (options().includes(by()) ? by() : "change");
  const groups = createMemo(() => grouped(pictures(), grouping(), filter()));
  const isOpen = (g: Group) => !g.folded || unfolded().has(g.key);
  // The order j and k step through: what shows, folded groups left out until opened.
  const order = createMemo(() => groups().flatMap((g) => (isOpen(g) ? g.pictures : [])));
  const current = createMemo<Shown | undefined>(() => {
    const key = picked();
    return order().find((p) => p.key === key) ?? pictures().find((p) => p.key === key) ?? order()[0];
  });
  const currentKey = createMemo(() => current()?.key);
  // The note being drawn, and where a film's clock is, for the picture on the stage.
  const [draft, setDraft] = createSignal<Draft>();
  const [composing, setComposing] = createSignal(false);
  const [time, setTime] = createSignal<number>();
  createEffect(
    on(currentKey, () => {
      setDraft(undefined);
      setComposing(false);
    }),
  );
  const notes = createMemo<Noted[]>(() => {
    const p = current();
    return p ? notesOn(props.state, p.check, p.picture.name, props.run) : [];
  });
  // Notes may be made on any run, a local one too: that is how they reach the agent early.
  const canNote = () => canSay(props.viewer);
  const noting: Noting = {
    get notes() {
      return notes();
    },
    get draft() {
      return draft();
    },
    get can() {
      return canNote() && mode() === "side";
    },
    onDraft: (next, done) => {
      setDraft(next && draft()?.both ? { ...next, both: true } : next);
      setComposing(!!next && (done || composing()));
      if (done) queueMicrotask(() => document.querySelector<HTMLElement>("textarea[data-words]")?.focus());
    },
  };
  const checks = createMemo(() => new Set(pictures().map((p) => p.check)).size);
  const step = (by: number) => {
    const list = order();
    const at = list.findIndex((p) => p.key === current()?.key);
    const next = list[Math.max(0, Math.min(list.length - 1, at + by))];
    if (next) pick(next.key);
  };
  const pick = (key: string) => {
    setPicked(key);
    document.querySelector(`[data-picture="${CSS.escape(key)}"]`)?.scrollIntoView({ block: "nearest" });
  };

  onMount(() => {
    const keys = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null;
      if (e.metaKey || e.ctrlKey || e.altKey || target?.closest("input, textarea, select, [contenteditable]")) return;
      if (e.key === "j") step(1);
      else if (e.key === "k") step(-1);
      else if (/^[1-5]$/.test(e.key)) setMode(MODES[Number(e.key) - 1]![0]);
      else return;
      e.preventDefault();
    };
    document.addEventListener("keydown", keys);
    onCleanup(() => document.removeEventListener("keydown", keys));
  });

  return (
    <section class="pictures" data-pictures aria-labelledby="pictures-title">
      <div class="pictures-head">
        <h2 id="pictures-title" class="visually-hidden">Pictures</h2>
        <nav class="counts" aria-label="Show only">
          <For each={counts(pictures())}>
            {(c) => (
              <button
                type="button"
                class={`count ${c.change}`}
                data-filter={c.change}
                aria-pressed={filter() === c.change}
                title={filter() === c.change ? "Show all" : `Show only ${WORDS[c.change].toLowerCase()}`}
                onClick={() => setFilter(filter() === c.change ? null : c.change)}
              >
                <b class="num">{c.n}</b>
                <span>{WORDS[c.change].toLowerCase()}</span>
              </button>
            )}
          </For>
        </nav>
        <Show when={options().length > 1}>
          <div class="seg group-by" role="group" aria-label="Group by">
            <For each={options()}>
              {(option) => (
                <button
                  type="button"
                  data-group={option}
                  aria-pressed={grouping() === option}
                  title={`Group by ${GROUP_WORDS[option].toLowerCase()}`}
                  onClick={() => setBy(option)}
                >
                  {GROUP_WORDS[option]}
                </button>
              )}
            </For>
          </div>
        </Show>
        <Show when={localBy(props.run)}>
          {(who) => (
            <p class="unproven" data-unproven title="A local run is the agent's word: notes reach the agent now, verdicts wait for CI">
              {who()}'s local run, notes only
            </p>
          )}
        </Show>
      </div>
      <div class="navigator">
        <For each={groups()}>
          {(group) => {
            const list = () => (
              <ul class="sheet">
                <For each={group.pictures}>
                  {(p) => (
                    <li>
                      <button
                        type="button"
                        class="thumb"
                        data-picture={p.key}
                        data-change={p.picture.change}
                        data-flaky={String(!!p.picture.flaky)}
                        data-verdict={standingOf(props.state, props.run, p)}
                        aria-current={current()?.key === p.key ? "true" : undefined}
                        title={`${title(p.parts)}: ${WORDS[p.picture.change].toLowerCase()}${p.picture.flaky ? ", flaky" : ""}${checks() > 1 ? `, ${p.check}` : ""}`}
                        onClick={() => pick(p.key)}
                      >
                        <span class="shot">
                          <Thumb shown={p} files={files()} />
                          <Switch>
                            <Match when={standingOf(props.state, props.run, p) === "looks_right"}>
                              <span class="right" aria-label="looks right" />
                            </Match>
                            <Match when={notesOn(props.state, p.check, p.picture.name, props.run).length}>
                              {(n) => (
                                <span class="noted" aria-label={`needs change, ${n()} open ${n() === 1 ? "note" : "notes"}`}>
                                  {n()}
                                </span>
                              )}
                            </Match>
                          </Switch>
                          <Show when={isFilm(p.picture)}>
                            <span class="film-mark" aria-label="film" />
                          </Show>
                        </span>
                        <span class="name">
                          <span class={`dot ${p.picture.change}`} aria-hidden="true" />
                          {title(p.parts)}
                        </span>
                      </button>
                    </li>
                  )}
                </For>
              </ul>
            );
            return (
              <Show
                when={group.folded}
                fallback={
                  <div class="group" data-group-key={group.key} data-group-label={group.label}>
                    <h3 class={group.change ?? ""}>{group.label}</h3>
                    {list()}
                  </div>
                }
              >
                <details
                  class="group"
                  data-group-key={group.key}
                  data-group-label={group.label}
                  open={unfolded().has(group.key)}
                  onToggle={(e) => {
                    const open = e.currentTarget.open;
                    setUnfolded((s) => {
                      const next = new Set(s);
                      if (open) next.add(group.key);
                      else next.delete(group.key);
                      return next;
                    });
                  }}
                >
                  <summary>
                    <h3 class={group.change ?? ""}>
                      {group.label} <span class="n num">{group.pictures.length}</span>
                    </h3>
                  </summary>
                  {list()}
                </details>
              </Show>
            );
          }}
        </For>
      </div>
      {/* A new stage per picture, and the same one while live events redraw its picture. */}
      <Show when={currentKey()} keyed>
        {(_key) => (
          <Stage
            shown={current()!}
            files={files()}
            head={props.run.head}
            mode={mode()}
            onMode={setMode}
            checks={checks()}
            noting={noting}
            onTime={setTime}
          />
        )}
      </Show>
      <Show when={current()}>
        {(p) => (
          <ReviewBar
            shown={p()}
            run={props.run}
            state={props.state}
            viewer={props.viewer}
            repository={props.state.work.repository}
            notes={notes()}
            canNote={canNote()}
            sideBySide={mode() === "side"}
            draft={composing() ? draft() : undefined}
            time={isFilm(p().picture) ? time() : undefined}
            position={order().findIndex((o) => o.key === p().key) + 1}
            of={order().length}
            onStep={step}
            onDraft={(d) => {
              setDraft(d);
              setComposing(!!d);
            }}
          />
        )}
      </Show>
    </section>
  );
}

/** A picture in the sheet, small: its after, or its before when it is gone; a film's first
 * frame; a text's summary. Empty until the files origin grants the run's files. */
function Thumb(props: { shown: Shown; files: string }) {
  if (isText(props.shown.picture)) {
    return (
      <span class="text-thumb" data-text-thumb>
        {props.shown.picture.summary}
      </span>
    );
  }
  const path = () => props.shown.picture.after ?? props.shown.picture.before;
  const src = () => {
    const p = path();
    return p && props.files ? fileUrl(props.files, p) : "";
  };
  return (
    <Show when={src()} fallback={<span class="blank" />}>
      {(url) => (
        <Show when={isFilm(props.shown.picture)} fallback={<img src={url()} alt="" loading="lazy" decoding="async" draggable={false} />}>
          <video src={`${url()}#t=0.1`} muted playsinline preload="metadata" aria-hidden="true" />
        </Show>
      )}
    </Show>
  );
}

/** Under the picture on the stage: what this person can say about it and where it stands, the
 * way to the next picture, the notes on it, and the note being written. */
function ReviewBar(props: {
  shown: Shown;
  run: Run;
  state: Snapshot;
  viewer: Viewer | undefined;
  repository: string;
  notes: Noted[];
  canNote: boolean;
  sideBySide: boolean;
  draft: Draft | undefined;
  time: number | undefined;
  position: number;
  of: number;
  onStep: (by: number) => void;
  onDraft: (draft: Draft | undefined) => void;
}) {
  const [error, setError] = createSignal("");
  const [sending, setSending] = createSignal<"looks_right" | "note" | null>(null);
  const [words, setWords] = createSignal("");
  const verdict = createMemo(() => verdictOf(props.state, props.run, props.shown));
  // Why this picture gets no verdict, in a few words; the reason in full is its title.
  const never = (): [string, string] | undefined => {
    if (localBy(props.run)) return ["Verdicts on CI", "A local run is unproven, so its pictures get a verdict on CI."];
    if (props.shown.picture.flaky) return ["Flaky", "Flaky pictures change from run to run, so they never wait for a verdict."];
    if (!toLookAt(props.shown)) return ["Unchanged", "Unchanged pictures need no verdict."];
    return undefined;
  };
  const standing = createMemo(() => standingOf(props.state, props.run, props.shown));
  const state = () => (never() ? "never" : standing());
  const both = () => props.shown.picture.before != null && props.shown.picture.after != null;
  const send = async () => {
    setSending("looks_right");
    setError((await say(props.run.id, props.shown)) ?? "");
    setSending(null);
  };
  const whole = () => {
    const side = props.shown.picture.after ? "after" : "before";
    props.onDraft({ side, region: { x: 0, y: 0, w: 1, h: 1 } });
    queueMicrotask(() => document.querySelector<HTMLElement>("textarea[data-words]")?.focus());
  };
  const add = async (e: Event) => {
    e.preventDefault();
    const d = props.draft;
    const text = words().trim();
    if (!d || !text) return;
    setSending("note");
    const refused = await makeNote(props.run.id, {
      check: props.shown.check,
      picture: props.shown.picture.name,
      side: sidesOf(d, !!d.both),
      region: d.region,
      ...(props.time != null ? { time: Math.round(props.time * 100) / 100 } : {}),
      words: text,
    });
    setSending(null);
    setError(refused ?? "");
    if (!refused) {
      setWords("");
      props.onDraft(undefined);
    }
  };
  const resolve = async (id: string) => {
    setError((await resolveNote(id)) ?? "");
  };
  const noteHint = () =>
    isText(props.shown.picture)
      ? "Note what should change in this text"
      : props.sideBySide
      ? `Drag on ${both() ? "either picture" : "the picture"} to mark what should change, or note the whole picture`
      : "Side by side, drag on a picture to mark what should change; or note the whole picture";
  return (
    <div class="verdict" data-said={state()} aria-live="polite">
      <div class="decide">
        <Switch>
          <Match when={never()}>{(why) => <p class="hint" title={why()[1]}>{why()[0]}</p>}</Match>
          <Match when={onlyViews(props.viewer)}>
            <p class="hint" title="Verdicts are given on the work's own page">
              Only views
            </p>
          </Match>
          <Match when={canSay(props.viewer)}>
            <button
              type="button"
              class="btn ok"
              data-say="looks_right"
              aria-pressed={!!verdict()}
              disabled={!!sending()}
              title={
                props.notes.length
                  ? "It needs change until its notes are resolved; looks right holds once they are, as long as it stays the same"
                  : "Looks right holds as long as it stays the same"
              }
              onClick={() => void send()}
            >
              Looks right
            </button>
          </Match>
          <Match when={props.viewer?.login}>
            <p class="hint" data-cannot title={`Giving a verdict needs write access to ${props.repository}`}>
              Needs write access
            </p>
          </Match>
          <Match when={maySignIn(props.viewer)}>
            <a class="plain" href={signIn()} data-sign-in title="Sign in to say whether it looks right">
              Sign in to review
            </a>
          </Match>
        </Switch>
        <Show when={props.canNote && !props.draft}>
          <button type="button" class="btn" data-whole title={noteHint()} onClick={whole}>
            Note
          </button>
        </Show>
        <Show when={standing() === "needs_change"}>
          <p class="said bad" title={`${props.notes.length} open ${props.notes.length === 1 ? "note" : "notes"}`}>
            <span class="needs" aria-hidden="true" />
            <b>Needs change</b>
          </p>
        </Show>
        <Show when={standing() === "looks_right" && verdict()}>
          {(v) => (
            <p class="said ok" title={[ago(Date.now() / 1000 - v().at), carried(v(), props.run)].filter(Boolean).join(", ")}>
              <span class="right" aria-hidden="true" />
              <span>
                by <b>{v().by}</b>
              </span>
            </p>
          )}
        </Show>
        <span class="step">
          <button type="button" class="ghost" aria-label="Previous picture" title="Previous (k)" onClick={() => props.onStep(-1)} disabled={props.position <= 1}>
            ‹
          </button>
          <span class="num">
            {props.position} of {props.of}
          </span>
          <button type="button" class="ghost" aria-label="Next picture" title="Next (j)" onClick={() => props.onStep(1)} disabled={props.position >= props.of}>
            ›
          </button>
        </span>
      </div>
      <Show when={props.notes.length}>
        <ol class="note-list" data-notes aria-label="Notes on this picture">
          <For each={props.notes}>
            {(n) => (
              <li data-note={n.note.id} data-n={n.n} data-old={String(n.old)} class={n.old ? "old" : ""}>
                <span class="n" aria-hidden="true">
                  {n.n}
                </span>
                <p class="words">
                  {n.note.words}
                  <span class="who" title={ago(Date.now() / 1000 - n.note.at)}>
                    {n.note.by}, {n.note.side === "both" ? "both sides" : n.note.side}
                    <Show when={n.note.time != null}>, at {n.note.time!.toFixed(2)} s</Show>
                    <Show when={fromWhere(n.note, props.run)}>{(where) => <>, {where()}</>}</Show>
                  </span>
                </p>
                <Show when={props.canNote}>
                  <button type="button" class="plain resolve" data-resolve onClick={() => void resolve(n.note.id)}>
                    Resolve
                  </button>
                </Show>
              </li>
            )}
          </For>
        </ol>
      </Show>
      <Show when={props.canNote && props.draft}>
        {(d) => (
          <form class="compose" data-compose onSubmit={(e) => void add(e)}>
            <label class="words-field">
              <span class="label">
                Note {props.notes.length + 1}
                <Show when={props.time != null}> at {props.time!.toFixed(2)} s</Show>
              </span>
              <textarea
                data-words
                rows="2"
                maxLength={2000}
                placeholder="What should change? Drag on the picture to point at it."
                value={words()}
                onInput={(e) => setWords(e.currentTarget.value)}
                onKeyDown={(e) => {
                  if (e.key === "Escape") props.onDraft(undefined);
                  else if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) void add(e);
                }}
              />
            </label>
            <div class="act">
              <button type="submit" class="btn" data-add-note disabled={!words().trim() || sending() === "note"}>
                Add note
              </button>
              <Show when={both()}>
                <label class="check">
                  <input
                    type="checkbox"
                    data-both
                    checked={!!d().both}
                    onChange={(e) => props.onDraft({ ...d(), both: e.currentTarget.checked })}
                  />
                  Both sides
                </label>
              </Show>
              <button type="button" class="plain" data-cancel-note onClick={() => props.onDraft(undefined)}>
                Cancel
              </button>
            </div>
          </form>
        )}
      </Show>
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
    </div>
  );
}
