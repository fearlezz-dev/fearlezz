// The page of a work: its title, where the run it shows stands and what waits for the person
// looking, what the change weakens, its pictures, and each run with its checks, live.

import { For, Match, Show, Switch, createEffect, createMemo, createSignal, on, onCleanup } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import { type CheckState, Feed, type Status, connect, stateOf } from "./live";
import { PicturesPanel } from "./PicturesPanel";
import { picturesOf } from "./pictures";
import type { Job, Review, Run, RunCheck, Snapshot, Weakening } from "./proof";
import { diff } from "./diff";
import { type Viewer, accept, canAccept, canSay, failedFor, localBy, maySignIn, signIn, standing, toLookAt, viewerOf, waitsFor } from "./review";
import { ShareControl } from "./ShareControl";
import { ago } from "./ago";

const WORDS: Record<CheckState, string> = {
  planned: "waiting",
  running: "running",
  passed: "passed",
  failed: "failed",
  not_run: "not run",
};

/** A work's page; through a share link (`shared`, its token) it only views. */
export function App(props: { shared?: string | undefined }) {
  const [status, setStatus] = createSignal<Status["kind"]>("connecting");
  const [gone, setGone] = createSignal("");
  const [work, setWork] = createStore<{ state?: Snapshot }>({});
  // The run a person picked; until then the newest, so a new run shows as it opens.
  const [picked, setPicked] = createSignal<string | null>(null);
  // Running checks show their time as it passes.
  const [now, setNow] = createSignal(Date.now() / 1000);
  const tick = setInterval(() => setNow(Date.now() / 1000), 1000);
  const stop = connect(location.pathname, () => new Feed(), (next: Status) => {
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
  // Through a share link the page only views, whoever looks: the host is not asked.
  createEffect(
    on(
      repository,
      (repo) => {
        if (props.shared) setViewer({ login: null, shared: true });
        else if (repo) void viewerOf(repo).then(setViewer);
      },
      { defer: false },
    ),
  );
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
                <div class="crumb">
                  <Show when={!props.shared} fallback={<span>{state().work.repository}</span>}>
                    <a href={`/${state().work.repository}`} data-repository>
                      {state().work.repository}
                    </a>
                  </Show>
                  <code class="branch">{state().work.branch}</code>
                  <span class={`live ${status()}`} title={status() === "live" ? "Updates live" : "Reconnecting"}>
                    {status() === "live" ? "live" : "reconnecting"}
                  </span>
                  <Show when={!props.shared && canSay(viewer())}>
                    <ShareControl work={state().work} />
                  </Show>
                </div>
                <h1 data-title>{title(state())}</h1>
                <div class="status">
                  <Show when={shown()}>{(run) => <Standing run={run()} review={review()} viewer={viewer()} />}</Show>
                  <Show when={props.shared}>
                    <span class="shared" data-shared title="A shared link: this page only views">
                      Shared, only views
                    </span>
                  </Show>
                </div>
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
                    <For each={latest(state().runs)}>
                      {(run) => <RunView run={run} now={now()} shown={shown()?.id === run.id} onShow={() => setPicked(run.id)} />}
                    </For>
                    <Show when={earlier(state().runs).length}>
                      {(n) => (
                        <details class="earlier" open={earlier(state().runs).some((r) => r.id === shown()?.id)}>
                          <summary>
                            {n()} earlier {n() === 1 ? "run" : "runs"}
                          </summary>
                          <For each={earlier(state().runs)}>
                            {(run) => (
                              <RunView run={run} now={now()} shown={shown()?.id === run.id} onShow={() => setPicked(run.id)} />
                            )}
                          </For>
                        </details>
                      )}
                    </Show>
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

/** Where the run shown stands, in one line: once it ran, what waits for the person looking. A
 * local run is the agent's word, so it says so first and waits for no one. */
function Standing(props: { run: Run; review: Review | undefined; viewer: Viewer | undefined }) {
  const ended = () => props.run.checks.filter((c) => c.entry).length;
  const failed = () => props.run.checks.filter((c) => c.entry?.outcome === "failed").length;
  const toLook = () => picturesOf(props.run).filter(toLookAt).length;
  const head = () => props.run.head.slice(0, 7);
  const kind = () => {
    if (localBy(props.run)) return "local";
    if (props.run.state === "running") return "run";
    const state = props.review?.state;
    // A weakening that waits for an accept is a decision, not a failure: it reads as one.
    if (state === "waits" || (state === "failed" && props.review?.weakened.length && !props.review.accept)) return "look";
    if (state === "needs_change") return "bad";
    if (state === "approved") return "ok";
    if (failed() || state === "failed" || props.run.state === "cancelled") return "bad";
    return toLook() ? "look" : "ok";
  };
  const pictures = (n: number) => `${n} ${n === 1 ? "picture" : "pictures"}`;
  const local = () => {
    const by = localBy(props.run);
    const what = `Unproven: ${by}'s local run of ${head()}`;
    if (props.run.state === "running") return `${what}, ${ended()} of ${props.run.checks.length} checks`;
    if (props.run.state === "cancelled") return `${what} stopped without its proof`;
    if (failed()) return `${what}: ${failed()} ${failed() === 1 ? "check" : "checks"} failed`;
    if (toLook()) return `${what}: ${pictures(toLook())} changed`;
    return `${what}: its checks passed`;
  };
  const text = () => {
    if (localBy(props.run)) return local();
    if (props.run.state === "running") {
      const looking = toLook() ? `, ${pictures(toLook())} to look at already` : "";
      return `Proving ${head()}: ${ended()} of ${props.run.checks.length} checks${looking}`;
    }
    if (props.run.state === "cancelled") return `The run of ${head()} ended without its proof`;
    if (failed()) return `${failed()} ${failed() === 1 ? "check" : "checks"} failed at ${head()}`;
    const review = props.review;
    if (review?.state === "failed") return failedFor(review, props.viewer);
    if (review?.state === "needs_change") return standing(review);
    if (review?.state === "waits") return waitsFor(review, props.viewer);
    if (review?.state === "approved") {
      return review.pictures.length || review.weakened.length
        ? `Everything looks right at ${head()}`
        : `Proved ${head()}`;
    }
    if (toLook()) return `${pictures(toLook())} changed at ${head()}`;
    return `Proved ${head()}`;
  };
  // The word on the pill; the sentence it shortens is its title.
  const word = () => {
    const by = localBy(props.run);
    if (by) return props.run.state === "running" ? "Running locally" : "Unproven";
    if (props.run.state === "running") return `Proving ${ended()} of ${props.run.checks.length}`;
    if (props.run.state === "cancelled") return "Stopped";
    if (failed()) return `${failed()} failed`;
    const review = props.review;
    if (review?.state === "failed") {
      if (!review.weakened.length || review.accept) return "Failed";
      return canAccept(props.viewer) ? "Waits for you" : "Waits for an admin";
    }
    if (review?.state === "needs_change") return "Needs change";
    if (review?.state === "waits") return canSay(props.viewer) ? "Waits for you" : "Waits for a look";
    if (review?.state === "approved") return review.pictures.length || review.weakened.length ? "Looks right" : "Proved";
    if (toLook()) return "Pictures changed";
    return "Proved";
  };
  return (
    <p class={`standing ${kind()}`} data-standing={kind()} title={text()}>
      <span class="pill">{word()}</span>
      <code class="head">{head()}</code>
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
      <div class="weakenings-head">
        <h2 id="weakenings-title">Weaker proof</h2>
        <Switch>
          <Match when={props.review.accept}>
            {(a) => (
              <p class="said ok" title={ago(Date.now() / 1000 - a().at)}>
                <span class="right" aria-hidden="true" /> Accepted by <b>{a().by}</b>
              </p>
            )}
          </Match>
          <Match when={canAccept(props.viewer)}>
            <button
              type="button"
              class="btn ok"
              data-accept
              disabled={sending() || !run()}
              title={`Accepts exactly these weakenings of ${props.review.head.slice(0, 7)} and runs its proof job again`}
              onClick={() => void send()}
            >
              Accept
            </button>
          </Match>
          <Match when={props.viewer?.login}>
            <p class="hint" data-cannot title={`Only an admin of ${props.repository} can accept a weaker proof`}>
              Needs an admin
            </p>
          </Match>
          <Match when={maySignIn(props.viewer)}>
            <a class="plain" href={signIn()} data-sign-in title={`Accepting needs admin rights on ${props.repository}`}>
              Sign in to accept
            </a>
          </Match>
        </Switch>
      </div>
      <ul>
        <For each={props.review.weakened}>{(w) => <WeakeningRow weakening={w} />}</For>
      </ul>
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
    </section>
  );
}

/** One weakened part: its name and each way it weakened, and on request the change itself,
 * its part of the config on the base and on the head, and why it waits for a person. */
function WeakeningRow(props: { weakening: Weakening }) {
  const [open, setOpen] = createSignal(false);
  const w = () => props.weakening;
  const name = () => {
    const weakening = w();
    return "check" in weakening ? weakening.check : weakening.setting;
  };
  const shown = () => w().was != null || w().now != null;
  return (
    <li data-weakening data-open={String(open())}>
      <div class="weakening-row">
        <code class="name">{name()}</code>
        <For each={w().hows}>
          {(how, i) => (
            <span class="how" data-how>
              {how.replace(/^it /, "")}
              {i() < w().hows.length - 1 ? "," : ""}
            </span>
          )}
        </For>
        <Show when={shown()}>
          <button type="button" class="plain" data-show-change aria-expanded={open()} onClick={() => setOpen(!open())}>
            {open() ? "Hide change" : "Show change"}
          </button>
        </Show>
      </div>
      <Show when={open()}>
        <div class="change">
          <Diff was={w().was} now={w().now} />
          <p class="why">Fearlezz can't tell whether this proves as much as before. Accept only if it does.</p>
        </div>
      </Show>
    </li>
  );
}

/** A part of the config before and after, line by line, the words that differ marked. */
function Diff(props: { was: string | null | undefined; now: string | null | undefined }) {
  return (
    <pre class="diff-lines" data-diff>
      <For each={diff(props.was, props.now)}>
        {(line) => (
          <span class={`line ${line.kind}`} data-line={line.kind}>
            <span class="sign" aria-hidden="true">
              {line.kind === "same" ? " " : line.kind === "was" ? "-" : "+"}
            </span>
            <For each={line.pieces}>{(p) => (p.differs ? <mark>{p.text}</mark> : p.text)}</For>
          </span>
        )}
      </For>
    </pre>
  );
}

/** The runs of the newest head, which the side column shows first; older ones fold away. */
const latest = (runs: Run[]) => runs.filter((r) => r.head === runs[0]?.head);
const earlier = (runs: Run[]) => runs.filter((r) => r.head !== runs[0]?.head);

/** A CI job GitHub says waits for a runner, before Fearlezz in it opens a run. */
function Waiting(props: { job: Job; now: number }) {
  return (
    <section class="run waiting" data-waiting={props.job.name}>
      <div class="run-head" title={`Waits for a runner since ${ago(props.now - props.job.since)}, ${props.job.head.slice(0, 7)}`}>
        <span class="run-title">
          <b>{props.job.name}</b>
          <span class="count">queued</span>
        </span>
        <span class="bar" aria-hidden="true" />
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
        <p class="pictures empty">
          {props.run.state === "running" ? "Pictures show here as checks take them." : "No pictures in this run."}
        </p>
      }
    >
      <PicturesPanel run={props.run} state={props.state} viewer={props.viewer} />
    </Show>
  );
}

/** A run in one line: its job (or who ran it locally), how many checks ended, and a bar with a
 * cell per check in its state. The run shown lists its checks. */
function RunView(props: { run: Run; now: number; shown: boolean; onShow: () => void }) {
  const done = () => props.run.checks.filter((c) => c.entry).length;
  const by = () => localBy(props.run);
  const name = () => (props.run.source.kind === "ci" ? props.run.source.job : "local");
  // A local run proves nothing: it ran, it did not prove.
  const verb = () => {
    if (props.run.state === "cancelled") return "Stopped";
    if (by()) return props.run.state === "running" ? "Running" : "Ran";
    return props.run.state === "running" ? "Proving" : "Proved";
  };
  const about = () =>
    `${verb()} ${props.run.head.slice(0, 7)}${by() ? `, local, by ${by()}` : ""}, ${ago(props.now - props.run.opened)}`;
  return (
    <section
      class={`run ${by() ? "local" : ""}`}
      data-run={props.run.id}
      data-state={props.run.state}
      data-source={props.run.source.kind}
      data-shown={String(props.shown)}
    >
      <button type="button" class="run-head" aria-pressed={props.shown} title={about()} onClick={() => props.onShow()}>
        <span class="run-title">
          <b>{name()}</b>
          <Show when={by()}>
            {(who) => (
              <span class="tag unproven" data-unproven title="A local run is the agent's word: it is never reviewed">
                {who()}
              </span>
            )}
          </Show>
          <span class="count num">
            {props.run.state === "cancelled" ? "stopped" : `${done()}/${props.run.checks.length}`}
          </span>
        </span>
        <span class="bar" aria-hidden="true">
          <For each={props.run.checks}>{(check) => <i class={stateOf(check)} />}</For>
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
                  Open on GitHub
                </a>
              )}
            </Show>
          )}
        </Show>
      </Show>
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
      <span class={`dot ${state()}`} role="img" aria-label={WORDS[state()]} title={WORDS[state()]} />
      <span class="id">{props.check.id}</span>
      <span class="seconds">{seconds()}</span>
    </li>
  );
}
