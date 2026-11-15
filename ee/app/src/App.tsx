// The page of a work: its title, where the run it shows stands and what waits for the person
// looking, what the change weakens, its pictures, and each run with its checks, live.

import { For, Match, Show, Switch, createEffect, createMemo, createSignal, on, onCleanup } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import { type CheckState, type Status, connect, stateOf } from "./live";
import { PicturesPanel } from "./PicturesPanel";
import { picturesOf } from "./pictures";
import type { Job, Review, Run, RunCheck, Snapshot } from "./proof";
import { type Viewer, accept, canAccept, signIn, viewerOf, waitsFor } from "./review";

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
  // Who looks, asked once the work says its repository.
  const [viewer, setViewer] = createSignal<Viewer>();
  const repository = createMemo(() => work.state?.work.repository);
  createEffect(on(repository, (repo) => repo && void viewerOf(repo).then(setViewer), { defer: false }));
  // The review, when it is of the head of the run shown.
  const review = createMemo(() => {
    const r = work.state?.review;
    return r && r.head === shown()?.head ? r : undefined;
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
                <Show when={shown()}>{(run) => <Standing run={run()} review={review()} viewer={viewer()} />}</Show>
              </header>
              <Show when={state().runs.length > 0} fallback={<p class="note">No runs yet.</p>}>
                <div class="cols">
                  <div class="main">
                    <Show when={review()?.weakened.length ? review() : undefined}>
                      {(r) => <Weakenings review={r()} runs={state().runs} viewer={viewer()} repository={state().work.repository} />}
                    </Show>
                    <Show when={shown()} keyed>
                      {(run) => <Looks run={run} state={state()} viewer={viewer()} />}
                    </Show>
                  </div>
                  <aside class="runs" aria-label="Runs">
                    <h2>Runs</h2>
                    <For each={state().waiting ?? []}>{(job) => <Waiting job={job} now={now()} />}</For>
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

/** Where the run shown stands, in one line: once it ran, what waits for the person looking. */
function Standing(props: { run: Run; review: Review | undefined; viewer: Viewer | undefined }) {
  const ended = () => props.run.checks.filter((c) => c.entry).length;
  const failed = () => props.run.checks.filter((c) => c.entry?.outcome === "failed").length;
  const toLook = () => picturesOf(props.run).filter((p) => p.picture.change !== "same" && !p.picture.flaky).length;
  const head = () => props.run.head.slice(0, 7);
  const kind = () => {
    if (props.run.state === "running") return "run";
    const state = props.review?.state;
    if (state === "waits") return "look";
    if (state === "approved") return "ok";
    if (failed() || state === "failed" || props.run.state === "cancelled") return "bad";
    return toLook() ? "look" : "ok";
  };
  const pictures = (n: number) => `${n} ${n === 1 ? "picture" : "pictures"}`;
  const text = () => {
    if (props.run.state === "running") {
      const looking = toLook() ? `, ${pictures(toLook())} to look at already` : "";
      return `Proving ${head()}: ${ended()} of ${props.run.checks.length} checks${looking}`;
    }
    if (props.run.state === "cancelled") return `The run of ${head()} ended without its proof`;
    if (failed()) return `${failed()} ${failed() === 1 ? "check" : "checks"} failed at ${head()}`;
    const review = props.review;
    if (review?.state === "failed") return `The proof of ${head()} failed`;
    if (review?.state === "waits") return waitsFor(review, props.viewer);
    if (review?.state === "approved") {
      return review.pictures.length || review.weakened.length
        ? `Everything looks right at ${head()}`
        : `Proved ${head()}`;
    }
    if (toLook()) return `${pictures(toLook())} changed at ${head()}`;
    return `Proved ${head()}`;
  };
  return (
    <p class={`standing ${kind()}`} data-standing={kind()}>
      {text()}
    </p>
  );
}

/** What the change weakens, waiting for an admin's accept of its head, and who can give it. */
function Weakenings(props: { review: Review; runs: Run[]; viewer: Viewer | undefined; repository: string }) {
  const [error, setError] = createSignal("");
  const [sending, setSending] = createSignal(false);
  // The newest finished CI run of the head: what the accept is of.
  const run = () => props.runs.find((r) => r.head === props.review.head && r.state === "done" && r.source.kind === "ci");
  const send = async () => {
    const target = run();
    if (!target) return;
    setSending(true);
    setError((await accept(target.id)) ?? "");
    setSending(false);
  };
  return (
    <section class="weakenings" data-weakenings data-accepted={String(!!props.review.accept)} aria-labelledby="weakenings-title">
      <h2 id="weakenings-title">What this change weakens</h2>
      <ul>
        <For each={props.review.weakened}>
          {(w) => (
            <li data-weakening>
              <b>
                {"check" in w ? (
                  <>
                    check <code>{w.check}</code> is weaker
                  </>
                ) : (
                  <>
                    <code>{w.setting}</code> is weaker
                  </>
                )}
              </b>
              : {w.how}
            </li>
          )}
        </For>
      </ul>
      <Switch>
        <Match when={props.review.accept}>
          {(a) => (
            <p class="said ok">
              Accepted by <b>{a().by}</b>, {ago(Date.now() / 1000 - a().at)}
            </p>
          )}
        </Match>
        <Match when={canAccept(props.viewer)}>
          <div class="act">
            <button type="button" class="btn ok" data-accept disabled={sending() || !run()} onClick={() => void send()}>
              Accept
            </button>
            <span class="hint">One accept covers every weakening of {props.review.head.slice(0, 7)}.</span>
          </div>
        </Match>
        <Match when={props.viewer?.login}>
          <p class="hint" data-cannot>
            Only an admin of {props.repository} can accept a weakened proof.
          </p>
        </Match>
        <Match when={props.viewer}>
          <p class="hint">
            <a href={signIn()} data-sign-in>
              Sign in
            </a>{" "}
            to accept it, if you are an admin of {props.repository}.
          </p>
        </Match>
      </Switch>
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
    </section>
  );
}

/** A CI job GitHub says waits for a runner, before Fearlezz in it opens a run. */
function Waiting(props: { job: Job; now: number }) {
  return (
    <section class="run waiting" data-waiting={props.job.name}>
      <div class="run-head">
        <span class="run-title">
          CI job {props.job.name} <span class="pill queued">queued</span>
        </span>
        <span class="run-meta">
          waiting for a runner, <code>{props.job.head.slice(0, 7)}</code>, {ago(props.now - props.job.since)}
        </span>
      </div>
    </section>
  );
}

/** The run's pictures, or what to expect of them. */
function Looks(props: { run: Run; state: Snapshot; viewer: Viewer | undefined }) {
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
      <PicturesPanel run={props.run} state={props.state} viewer={props.viewer} />
    </Show>
  );
}

function RunView(props: { run: Run; now: number; shown: boolean; onShow: () => void }) {
  const done = () => props.run.checks.filter((c) => c.entry).length;
  const failed = () => props.run.checks.some((c) => c.entry?.outcome === "failed");
  const pill = () =>
    props.run.state === "running" ? "running" : props.run.state === "cancelled" ? "cancelled" : failed() ? "failed" : "done";
  return (
    <section class="run" data-run={props.run.id} data-state={props.run.state} data-shown={String(props.shown)}>
      <button type="button" class="run-head" aria-pressed={props.shown} onClick={() => props.onShow()}>
        <span class="run-title">
          {props.run.state === "running" ? "Proving" : props.run.state === "cancelled" ? "Stopped" : "Proved"}{" "}
          <code>{props.run.head.slice(0, 7)}</code>
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
        <Show when={props.run.source.kind === "ci" ? props.run.source : undefined}>
          {(ci) => (
            <Show when={ci().run_url}>
              {(url) => (
                <a class="job" href={url()} rel="noreferrer">
                  CI job {ci().job}
                </a>
              )}
            </Show>
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
