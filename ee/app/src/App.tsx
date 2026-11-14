// The page of a work: its title, and each run with its checks, live.

import { For, Match, Show, Switch, createSignal, onCleanup } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import { type CheckState, type Status, connect, stateOf } from "./live";
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

  return (
    <main>
      <Switch>
        <Match when={status() === "gone"}>
          <p class="note">{gone()}</p>
        </Match>
        <Match when={work.state}>
          {(state) => (
            <>
              <p class="repo">
                {state().work.repository} <span class="branch">{state().work.branch}</span>
                <span class={`live ${status()}`}>{status() === "live" ? "live" : "reconnecting"}</span>
              </p>
              <h1 data-title>{title(state())}</h1>
              <Show when={state().runs.length > 0} fallback={<p class="note">No runs yet.</p>}>
                <For each={state().runs}>{(run) => <RunView run={run} now={now()} />}</For>
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

function RunView(props: { run: Run; now: number }) {
  const done = () => props.run.checks.filter((c) => c.entry).length;
  return (
    <section class="run" data-run={props.run.id} data-state={props.run.state}>
      <header>
        <h2>
          {props.run.state === "running" ? "Proving" : "Proved"} <code>{props.run.head.slice(0, 7)}</code>
        </h2>
        <span class={`pill ${props.run.state}`}>{props.run.state === "running" ? "running" : "done"}</span>
        <span class="count">
          {done()} of {props.run.checks.length} checks
        </span>
        <Show when={props.run.source.run_url}>
          {(url) => (
            <a class="job" href={url()} rel="noreferrer">
              CI job {props.run.source.job}
            </a>
          )}
        </Show>
      </header>
      <ul class="checks">
        <For each={props.run.checks}>{(check) => <CheckView check={check} now={props.now} />}</For>
      </ul>
    </section>
  );
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
