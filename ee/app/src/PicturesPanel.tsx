// A run's pictures: counts that filter, grouping by the parts of their names, every picture in
// the order that directs attention, the one on the stage and the verdict on it. j and k step
// between pictures, 1 to 5 pick the way to compare.

import { For, Match, Show, Switch, createMemo, createSignal, onCleanup, onMount } from "solid-js";
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
import type { PictureChange, Run, Snapshot } from "./proof";
import { type Viewer, canSay, carried, looksRight, signIn, toLookAt, verdictOf } from "./review";
import { MODES, type Mode, Stage } from "./Stage";

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
                        <Show when={verdictOf(props.state, props.run, p)}>
                          <span class="right" title="Looks right" aria-label="looks right" />
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
          />
        )}
      </Show>
      <Show when={current()}>
        {(p) => <VerdictBar shown={p()} run={props.run} state={props.state} viewer={props.viewer} repository={props.state.work.repository} />}
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

/** The verdict on the picture on the stage: who said it looks right and when, or what this
 * person can do about it. */
function VerdictBar(props: { shown: Shown; run: Run; state: Snapshot; viewer: Viewer | undefined; repository: string }) {
  const [error, setError] = createSignal("");
  const [sending, setSending] = createSignal(false);
  const verdict = createMemo(() => verdictOf(props.state, props.run, props.shown));
  const never = () => {
    if (props.run.source.kind !== "ci") return "A local run is never reviewed; its pictures get a verdict on CI.";
    if (props.shown.picture.flaky) return "Flaky pictures never wait for a verdict.";
    if (!toLookAt(props.shown)) return "Unchanged pictures need no verdict.";
    return "";
  };
  const state = () => (never() ? "never" : (verdict()?.kind ?? "none"));
  const send = async () => {
    setSending(true);
    setError((await looksRight(props.run.id, props.shown)) ?? "");
    setSending(false);
  };
  return (
    <div class="verdict" data-said={state()} aria-live="polite">
      <Switch>
        <Match when={never()}>
          <p class="hint">{never()}</p>
        </Match>
        <Match when={verdict()}>
          {(v) => (
            <p class="said ok">
              <span class="right" aria-hidden="true" />
              <span>
                <b>Looks right</b> to <b>{v().by}</b>, {ago(Date.now() / 1000 - v().at)}
                <Show when={carried(v(), props.run)}>
                  {(where) => <span class="carried">, {where()}</span>}
                </Show>
              </span>
            </p>
          )}
        </Match>
        <Match when={canSay(props.viewer)}>
          <div class="act">
            <button type="button" class="btn ok" data-say="looks_right" disabled={sending()} onClick={() => void send()}>
              Looks right
            </button>
            <span class="hint">It holds for this picture as long as it stays the same.</span>
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
