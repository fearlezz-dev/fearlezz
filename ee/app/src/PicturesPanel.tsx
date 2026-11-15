// A run's pictures: counts that filter, grouping by the parts of their names, every picture in
// the order that directs attention, the one on the stage, the notes on it and the verdict. j
// and k step between pictures, 1 to 5 pick the way to compare.

import { For, Match, Show, Switch, createEffect, createMemo, createSignal, on, onCleanup, onMount } from "solid-js";
import { grantOf } from "./grant";
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
  picturesOf,
  title,
} from "./pictures";
import { type Draft, type Shown as Noted, fromWhere, makeNote, notesOn, resolveNote, sidesOf } from "./notes";
import type { PictureChange, Run, Snapshot, VerdictKind } from "./proof";
import { type Viewer, canSay, carried, localBy, signIn, toLookAt, verdict as say, verdictOf } from "./review";
import { MODES, type Mode, type Noting, Stage } from "./Stage";

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
        <h2 id="pictures-title">Does it look right to you?</h2>
        <Show when={localBy(props.run)}>
          {(who) => (
            <p class="unproven" data-unproven>
              <b>{who()}'s local run</b> is the agent's word. Notes reach the agent now; verdicts wait for CI.
            </p>
          )}
        </Show>
        <nav class="counts" aria-label="Show only">
          <For each={counts(pictures())}>
            {(c) => (
              <button
                type="button"
                class={`count ${c.change}`}
                data-filter={c.change}
                aria-pressed={filter() === c.change}
                onClick={() => setFilter(filter() === c.change ? null : c.change)}
              >
                <b class="num">{c.n}</b>
                <span>{WORDS[c.change]}</span>
              </button>
            )}
          </For>
        </nav>
        <Show when={options().length > 1}>
          <div class="group-by">
            <span id="group-by">Group by</span>
            <div class="seg" role="group" aria-labelledby="group-by">
              <For each={options()}>
                {(option) => (
                  <button type="button" data-group={option} aria-pressed={grouping() === option} onClick={() => setBy(option)}>
                    {GROUP_WORDS[option]}
                  </button>
                )}
              </For>
            </div>
          </div>
        </Show>
      </div>
      <div class="navigator">
        <For each={groups()}>
          {(group) => {
            const list = () => (
              <ul class="chips">
                <For each={group.pictures}>
                  {(p) => (
                    <li>
                      <button
                        type="button"
                        class="chip-pic"
                        data-picture={p.key}
                        data-change={p.picture.change}
                        data-flaky={String(!!p.picture.flaky)}
                        data-verdict={verdictOf(props.state, props.run, p)?.kind ?? "none"}
                        aria-current={current()?.key === p.key ? "true" : undefined}
                        onClick={() => pick(p.key)}
                      >
                        <span class={`dot ${p.picture.change}`} aria-hidden="true" />
                        <span class="name">{title(p.parts)}</span>
                        <Show when={isFilm(p.picture)}>
                          <span class="mini">film</span>
                        </Show>
                        <Show when={p.picture.flaky}>
                          <span class="mini">flaky</span>
                        </Show>
                        <Switch>
                          <Match when={verdictOf(props.state, props.run, p)?.kind === "looks_right"}>
                            <span class="right" title="Looks right" aria-label="looks right" />
                          </Match>
                          <Match when={verdictOf(props.state, props.run, p)?.kind === "needs_change"}>
                            <span class="needs" title="Needs change" aria-label="needs change" />
                          </Match>
                        </Switch>
                        <Show when={notesOn(props.state, p.check, p.picture.name, props.run).length}>
                          {(n) => (
                            <span class="mini noted" aria-label={`${n()} open ${n() === 1 ? "note" : "notes"}`}>
                              {n()}
                            </span>
                          )}
                        </Show>
                        <Show when={checks() > 1 && grouping() !== "check"}>
                          <span class="mini">{p.check}</span>
                        </Show>
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
                    <h3 class={group.change ?? ""}>
                      {group.label} <span class="n num">{group.pictures.length}</span>
                    </h3>
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
        {(key) => (
          <Stage
            shown={current()!}
            files={files()}
            head={props.run.head}
            mode={mode()}
            onMode={setMode}
            position={order().findIndex((p) => p.key === key) + 1}
            of={order().length}
            onStep={step}
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
            onDraft={(d) => {
              setDraft(d);
              setComposing(!!d);
            }}
          />
        )}
      </Show>
      <details class="keys">
        <summary>Keys</summary>
        <p>
          <kbd>j</kbd> <kbd>k</kbd> next and previous picture, <kbd>1</kbd> to <kbd>5</kbd> how to compare
        </p>
      </details>
    </section>
  );
}

/** Under the picture on the stage: the notes on it, the note being written, and the verdict:
 * who said it looks right or needs change and when, or what this person can do about it. */
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
  onDraft: (draft: Draft | undefined) => void;
}) {
  const [error, setError] = createSignal("");
  const [sending, setSending] = createSignal<VerdictKind | "note" | null>(null);
  const [words, setWords] = createSignal("");
  const verdict = createMemo(() => verdictOf(props.state, props.run, props.shown));
  const never = () => {
    if (localBy(props.run)) return "A local run is unproven, so its pictures get a verdict on CI.";
    if (props.shown.picture.flaky) return "Flaky pictures never wait for a verdict.";
    if (!toLookAt(props.shown)) return "Unchanged pictures need no verdict.";
    return "";
  };
  const state = () => (never() ? "never" : (verdict()?.kind ?? "none"));
  const both = () => props.shown.picture.before != null && props.shown.picture.after != null;
  const send = async (kind: VerdictKind) => {
    setSending(kind);
    setError((await say(props.run.id, props.shown, kind)) ?? "");
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
  return (
    <div class="verdict" data-said={state()} aria-live="polite">
      <Show when={verdict()}>
          {(v) => (
            <p class={`said ${v().kind === "looks_right" ? "ok" : "bad"}`}>
              <span class={v().kind === "looks_right" ? "right" : "needs"} aria-hidden="true" />
              <span>
                <b>{v().kind === "looks_right" ? "Looks right" : "Needs change"}</b>, said by <b>{v().by}</b>, {ago(Date.now() / 1000 - v().at)}
                <Show when={carried(v(), props.run)}>
                  {(where) => <span class="carried">, {where()}</span>}
                </Show>
              </span>
            </p>
          )}
      </Show>
      <Show when={props.notes.length}>
        <ol class="note-list" data-notes aria-label="Notes on this picture">
          <For each={props.notes}>
            {(n) => (
              <li data-note={n.note.id} data-n={n.n} data-old={String(n.old)} class={n.old ? "old" : ""}>
                <span class="n" aria-hidden="true">
                  {n.n}
                </span>
                <div class="body">
                  <p class="words">{n.note.words}</p>
                  <p class="who">
                    {n.note.by}, {n.note.side === "both" ? "both sides" : n.note.side}
                    <Show when={n.note.time != null}>, at {n.note.time!.toFixed(2)} s</Show>, {ago(Date.now() / 1000 - n.note.at)}
                    <Show when={fromWhere(n.note, props.run)}>{(where) => <>, {where()}</>}</Show>
                  </p>
                </div>
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
      <Show when={props.canNote}>
        <Show
          when={props.draft}
          fallback={
            <div class="act note-start">
              <span class="hint">
                {props.sideBySide
                  ? both()
                    ? "Drag on either picture to mark what should change."
                    : "Drag on the picture to mark what should change."
                  : "Side by side, drag on a picture to mark what should change."}
              </span>
              <button type="button" class="plain" data-whole onClick={whole}>
                Note on the whole picture
              </button>
            </div>
          }
        >
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
                  placeholder="What should change here?"
                  value={words()}
                  onInput={(e) => setWords(e.currentTarget.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Escape") props.onDraft(undefined);
                    else if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) void add(e);
                  }}
                />
              </label>
              <div class="act">
                <Show when={both()}>
                  <label class="check">
                    <input
                      type="checkbox"
                      data-both
                      checked={!!d().both}
                      onChange={(e) => props.onDraft({ ...d(), both: e.currentTarget.checked })}
                    />
                    On both sides
                  </label>
                </Show>
                <button type="submit" class="btn" data-add-note disabled={!words().trim() || sending() === "note"}>
                  Add note
                </button>
                <button type="button" class="plain" data-cancel-note onClick={() => props.onDraft(undefined)}>
                  Cancel
                </button>
              </div>
            </form>
          )}
        </Show>
      </Show>
      <Switch>
        <Match when={never()}>
          <p class="hint">{never()}</p>
        </Match>
        <Match when={canSay(props.viewer)}>
          <div class="act verdicts">
            <button
              type="button"
              class="btn bad"
              data-say="needs_change"
              aria-pressed={verdict()?.kind === "needs_change"}
              disabled={!!sending() || props.notes.length === 0}
              onClick={() => void send("needs_change")}
            >
              Needs change
            </button>
            <button
              type="button"
              class="btn ok"
              data-say="looks_right"
              aria-pressed={verdict()?.kind === "looks_right"}
              disabled={!!sending()}
              onClick={() => void send("looks_right")}
            >
              Looks right
            </button>
            <span class="hint">
              {props.notes.length ? "A verdict holds for this picture as long as it stays the same." : "Needs change asks for a note first."}
            </span>
          </div>
        </Match>
        <Match when={props.viewer?.login}>
          <p class="hint" data-cannot>
            Giving a verdict needs write access to {props.repository}.
          </p>
        </Match>
        <Match when={props.viewer}>
          <p class="hint">
            <a href={signIn()} data-sign-in>
              Sign in
            </a>{" "}
            to say whether it looks right.
          </p>
        </Match>
      </Switch>
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
    </div>
  );
}

/** How long ago, roughly, as people say it. */
function ago(seconds: number): string {
  if (seconds < 60) return "just now";
  if (seconds < 3600) return `${Math.floor(seconds / 60)} min ago`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)} h ago`;
  return `${Math.floor(seconds / 86400)} days ago`;
}
