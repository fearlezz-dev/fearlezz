// The page of a work: its title, where the run it shows stands, its pictures, and each run with
// its checks, live.

import { For, Match, Show, Switch, createMemo, createSignal, onCleanup } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import { type CheckState, type Status, connect, stateOf } from "./live";
import { PicturesPanel } from "./PicturesPanel";
import { picturesOf } from "./pictures";
import type { Run, RunCheck, Snapshot } from "./proof";

const WORDS: Record<CheckState, string> = {
  planned: "waiting",
  running: "running",
  passed: "passed",
  failed: "failed",
  not_run: "not run",
};

export function App() {
  const [status, setStatus] = createSignal<Status["kind"]>("connecting");
  const [gone, setGone] = createSignal("");
  const [work, setWork] = createStore<{ state?: Snapshot }>({});
  // The run a person picked; until then the newest, so a new run shows as it opens.
  const [picked, setPicked] = createSignal<string | null>(null);
  // Running checks show their time as it passes.
  const [now, setNow] = createSignal(Date.now() / 1000);
  const tick = setInterval(() => setNow(Date.now() / 1000), 1000);
  const stop = connect(location.pathname, (next) => {
    setStatus(next.kind);
    if (next.kind === "live") setWork("state", reconcile(next.state, { key: "id" }));
    if (next.kind === "gone") setGone(next.why);
  });
  onCleanup(() => {
    stop();
    clearInterval(tick);
  });
  const shown = createMemo(() => {
    const runs = work.state?.runs ?? [];
    return runs.find((r) => r.id === picked()) ?? runs[0];
  });

  return (
    <main>
      <Switch>
        <Match when={status() === "gone"}>
          <p class="note">{gone()}</p>
        </Match>
        <Match when={work.state}>
          {(state) => (
            <>
              <header class="top">
                <p class="repo">
                  {state().work.repository} <span class="branch">{state().work.branch}</span>
                  <span class={`live ${status()}`}>{status() === "live" ? "live" : "reconnecting"}</span>
                </p>
                <h1 data-title>{title(state())}</h1>
                <Show when={shown()}>{(run) => <Standing run={run()} />}</Show>
              </header>
              <Show when={state().runs.length > 0} fallback={<p class="note">No runs yet.</p>}>
                <div class="cols">
                  <div class="main">
                    <Show when={shown()} keyed>
                      {(run) => <Looks run={run} />}
                    </Show>
                  </div>
                  <aside class="runs" aria-label="Runs">
                    <h2>Runs</h2>
                    <For each={state().runs}>
                      {(run) => (
                        <RunView
                          run={run}
                          now={now()}
                          shown={shown()?.id === run.id}
                          onShow={() => setPicked(run.id)}
                        />
                      )}
                    </For>
                  </aside>
                </div>
              </Show>
            </>
          )}
        </Match>
        <Match when={true}>
          <p class="note">Connecting…</p>
        </Match>
      </Switch>
    </main>
  );
}

function title(state: Snapshot): string {
  const pull = state.work.pull;
  if (!pull) return state.work.branch;
  return pull.title ? `#${pull.number} ${pull.title}` : `#${pull.number}`;
}

/** Where the run shown stands, in one line. */
function Standing(props: { run: Run }) {
  const ended = () => props.run.checks.filter((c) => c.entry).length;
  const failed = () => props.run.checks.filter((c) => c.entry?.outcome === "failed").length;
  const toLook = () => picturesOf(props.run).filter((p) => p.picture.change !== "same" && !p.picture.flaky).length;
  const head = () => props.run.head.slice(0, 7);
  const kind = () => (props.run.state === "running" ? "run" : failed() ? "bad" : toLook() ? "look" : "ok");
  const pictures = (n: number) => `${n} ${n === 1 ? "picture" : "pictures"}`;
  const text = () => {
    if (props.run.state === "running") {
      const looking = toLook() ? `, ${pictures(toLook())} to look at already` : "";
      return `Proving ${head()}: ${ended()} of ${props.run.checks.length} checks${looking}`;
    }
    if (failed()) return `${failed()} ${failed() === 1 ? "check" : "checks"} failed at ${head()}`;
    if (toLook()) return `${pictures(toLook())} changed at ${head()}`;
    return `Proved ${head()}`;
  };
  return (
    <p class={`standing ${kind()}`} data-standing={kind()}>
      {text()}
    </p>
  );
}

/** The run's pictures, or what to expect of them. */
function Looks(props: { run: Run }) {
  const any = () => props.run.checks.some((c) => c.look);
  return (
    <Show
      when={any()}
      fallback={
        <section class="pictures empty">
          <h2>Does it look right to you?</h2>
          <p class="note">
            {props.run.state === "running"
              ? "Pictures show here as the checks that take them finish."
              : "This run took no pictures."}
          </p>
        </section>
      }
    >
      <PicturesPanel run={props.run} />
    </Show>
  );
}

function RunView(props: { run: Run; now: number; shown: boolean; onShow: () => void }) {
  const done = () => props.run.checks.filter((c) => c.entry).length;
  const failed = () => props.run.checks.some((c) => c.entry?.outcome === "failed");
  const pill = () => (props.run.state === "running" ? "running" : failed() ? "failed" : "done");
  return (
    <section class="run" data-run={props.run.id} data-state={props.run.state} data-shown={String(props.shown)}>
      <button type="button" class="run-head" aria-pressed={props.shown} onClick={() => props.onShow()}>
        <span class="run-title">
          {props.run.state === "running" ? "Proving" : "Proved"} <code>{props.run.head.slice(0, 7)}</code>
          <span class={`pill ${pill()}`}>{pill()}</span>
        </span>
        <span class="run-meta">
          {done()} of {props.run.checks.length} checks, {ago(props.now - props.run.opened)}
        </span>
      </button>
      <Show when={props.shown}>
        <ul class="checks">
          <For each={props.run.checks}>{(check) => <CheckView check={check} now={props.now} />}</For>
        </ul>
        <Show when={props.run.source.run_url}>
          {(url) => (
            <a class="job" href={url()} rel="noreferrer">
              CI job {props.run.source.job}
            </a>
          )}
        </Show>
      </Show>
    </section>
  );
}

/** How long ago, roughly, as people say it. */
function ago(seconds: number): string {
  if (seconds < 60) return "just now";
  if (seconds < 3600) return `${Math.floor(seconds / 60)} min ago`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)} h ago`;
  return `${Math.floor(seconds / 86400)} days ago`;
}

function CheckView(props: { check: RunCheck; now: number }) {
  const state = () => stateOf(props.check);
  // Seconds it took, or, while it runs, how long it has run so far.
  const seconds = () => {
    const entry = props.check.entry;
    if (entry) return `${entry.seconds.toFixed(1)} s`;
    const started = props.check.started;
    return started == null ? "" : `${Math.max(0, props.now - started).toFixed(0)} s`;
  };
  return (
    <li data-check={props.check.id} data-state={state()}>
      <span class={`dot ${state()}`} aria-hidden="true" />
      <span class="id">{props.check.id}</span>
      <span class="state">{WORDS[state()]}</span>
      <span class="seconds">{seconds()}</span>
    </li>
  );
}
