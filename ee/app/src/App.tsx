// The page of a work: its title, where the run it shows stands and what waits for the person
// looking, what the change weakens, its pictures, and each run with its checks, live.

import { For, Match, Show, Switch, createEffect, createMemo, createSignal, on, onCleanup } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import { type CheckState, Feed, type Status, connect, namedTests, stateOf } from "./live";
import { PicturesPanel } from "./PicturesPanel";
import type { Job, Review, Run, RunCheck, Snapshot, Weakening } from "./proof";
import { DiffLines } from "./DiffLines";
import { Failures, failedOf } from "./Failures";
import { type Looker, accept, canAccept, canSay, localBy, maySignIn, opening, signIn, signedIn, viewerOf } from "./review";
import { ShareControl } from "./ShareControl";
import { type Crumb, Placeholder, TopBar } from "./Shell";
import { Icon, type IconName } from "./icons";
import { hoped, seen } from "./hopes";
import { headline } from "./standing";
import { ago } from "./ago";
import { short } from "./short";

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
  // The run a person picked; until then the one `opening` names, so a new run shows as it opens.
  const [picked, setPicked] = createSignal<string | null>(null);
  // Running checks show their time as it passes.
  const [now, setNow] = createSignal(Date.now() / 1000);
  const tick = setInterval(() => setNow(Date.now() / 1000), 1000);
  const stop = connect(location.pathname, () => new Feed(), (next: Status) => {
    setStatus(next.kind);
    if (next.kind === "live") {
      setWork("state", reconcile(next.state, { key: "id" }));
      seen();
    }
    if (next.kind === "gone") setGone(next.why);
  });
  onCleanup(() => {
    stop();
    clearInterval(tick);
  });
  // The work with what this person said that the host has not settled yet, for the pictures.
  const said = createMemo(() => work.state && hoped(work.state));
  const shown = createMemo(() => {
    const runs = work.state?.runs ?? [];
    return runs.find((r) => r.id === picked()) ?? opening(runs);
  });
  // Who looks, asked once the work says its repository.
  const [viewer, setViewer] = createSignal<Looker>();
  const repository = createMemo(() => work.state?.work.repository);
  // Through a share link the page only views, whoever looks: the host is not asked.
  createEffect(
    on(
      repository,
      (repo) => {
        if (props.shared) setViewer("shared");
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

  const crumbs = (): Crumb[] => {
    const w = work.state?.work;
    if (!w) return [];
    const pull = w.pull;
    return [
      { label: w.repository, href: props.shared ? undefined : `/${w.repository}`, data: "repository" },
      pull ? { label: `#${pull.number}` } : { label: w.branch, mono: true },
    ];
  };

  return (
    <>
      <TopBar crumbs={crumbs()} status={status()}>
        <Show when={!props.shared && canSay(viewer()) && work.state}>{(state) => <ShareControl work={state().work} />}</Show>
      </TopBar>
      <main>
        <Switch>
          <Match when={status() === "gone"}>
            <div class="empty" data-gone>
              <Icon name="alert" size={22} />
              <p class="note">{gone()}</p>
            </div>
          </Match>
          <Match when={work.state}>
            {(state) => (
              <>
                <header class="hero">
                  <h1 data-title>{title(state())}</h1>
                  <div class="meta">
                    <span class="meta-item">
                      <Icon name="branch" />
                      <code class="branch">{state().work.branch}</code>
                    </span>
                    <Show when={shown()}>
                      {(run) => (
                        <span class="meta-item" title="The commit the run shown proves">
                          <Icon name="commit" />
                          <code class="head">{short(run().head)}</code>
                        </span>
                      )}
                    </Show>
                    <Show when={state().work.pull?.url}>
                      {(url) => (
                        <a class="meta-item link" href={url()} target="_blank" rel="noopener noreferrer">
                          <Icon name="pull" />
                          Pull request
                        </a>
                      )}
                    </Show>
                    {/* The deploy its proof made of this head, which a person can open and try. */}
                    <Show when={review()?.head === shown()?.head && webAddress(review()?.preview)}>
                      {(url) => (
                        <a class="meta-item link" data-preview href={url()} target="_blank" rel="noopener noreferrer">
                          <Icon name="external" />
                          Preview
                        </a>
                      )}
                    </Show>
                    <Show when={props.shared}>
                      <span class="meta-item shared" data-shared title="A shared link: this page only views">
                        <Icon name="eye" />
                        Shared, only views
                      </span>
                    </Show>
                  </div>
                  <Show when={shown()}>{(run) => <Standing run={run()} review={review()} viewer={viewer()} />}</Show>
                </header>
                <Show
                  when={state().runs.length > 0}
                  fallback={
                    <div class="empty">
                      <Icon name="clock" size={22} />
                      <p class="note">No runs yet.</p>
                    </div>
                  }
                >
                  <div class="cols">
                    <div class="main">
                      <Show when={review()?.weakened.length ? review() : undefined}>
                        {(r) => <Weakenings review={r()} runs={state().runs} viewer={viewer()} repository={state().work.repository} />}
                      </Show>
                      <Show when={shown()} keyed>
                        {(run) => (
                          <>
                            <Show when={failedOf(run).length}>
                              <Failures checks={failedOf(run)} />
                            </Show>
                            <Looks run={run} state={said() ?? state()} viewer={viewer()} />
                          </>
                        )}
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
                            <div class="earlier-runs">
                              <For each={earlier(state().runs)}>
                                {(run) => (
                                  <RunView run={run} now={now()} shown={shown()?.id === run.id} onShow={() => setPicked(run.id)} />
                                )}
                              </For>
                            </div>
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
            <Placeholder />
          </Match>
        </Switch>
      </main>
    </>
  );
}

function title(state: Snapshot): string {
  const pull = state.work.pull;
  if (!pull) return state.work.branch;
  return pull.title ? `#${pull.number} ${pull.title}` : `#${pull.number}`;
}

/** Where the run shown stands, in one line, as `headline` words it; a bar while it runs. */
function Standing(props: { run: Run; review: Review | undefined; viewer: Looker | undefined }) {
  const line = () => headline(props.run, props.review, props.viewer);
  const ended = () => props.run.checks.filter((c) => c.entry).length;
  return (
    <div class={`standing ${line().tone}`} data-standing={line().tone} title={line().title} role="status">
      <span class="standing-icon">
        <Icon name={line().icon} size={18} />
      </span>
      <span class="pill">{line().word}</span>
      <span class="detail">{line().detail}</span>
      <Show when={props.run.state === "running"}>
        <span class="progress" aria-hidden="true">
          <i style={{ width: `${(ended() / Math.max(1, props.run.checks.length)) * 100}%` }} />
        </span>
      </Show>
    </div>
  );
}

/** The address itself when it is a plain web address; the host takes no other, and a link never
 * runs script. */
function webAddress(url: string | null | undefined): string | undefined {
  return url && /^https?:\/\/[^\s/]/.test(url) ? url : undefined;
}

/** What the change weakens, waiting for an admin's accept of its head, and who can give it. */
function Weakenings(props: { review: Review; runs: Run[]; viewer: Looker | undefined; repository: string }) {
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
        <h2 id="weakenings-title">
          <Icon name="shield" size={18} />
          Weaker proof
        </h2>
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
              title={`Accepts exactly these weakenings of ${short(props.review.head)} and runs its proof job again`}
              onClick={() => void send()}
            >
              Accept
            </button>
          </Match>
          <Match when={signedIn(props.viewer)}>
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
          <DiffLines was={w().was} now={w().now} />
          <p class="why">Fearlezz can't tell whether this proves as much as before. Accept only if it does.</p>
        </div>
      </Show>
    </li>
  );
}

/** The runs of the newest head, which the side column shows first; older ones fold away. */
const latest = (runs: Run[]) => runs.filter((r) => r.head === runs[0]?.head);
const earlier = (runs: Run[]) => runs.filter((r) => r.head !== runs[0]?.head);

/** A CI job GitHub says waits for a runner, before Fearlezz in it opens a run. */
function Waiting(props: { job: Job; now: number }) {
  return (
    <section class="run waiting" data-waiting={props.job.name}>
      <div class="run-head" title={`Waits for a runner since ${ago(props.now - props.job.since)}, ${short(props.job.head)}`}>
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
function Looks(props: { run: Run; state: Snapshot; viewer: Looker | undefined }) {
  const any = () => props.run.checks.some((c) => c.look);
  // A run that failed says why above; that it took no pictures is no news then.
  const empty = () => {
    if (props.run.state === "running") return "Pictures show here as checks take them.";
    return failedOf(props.run).length ? "" : "No pictures in this run.";
  };
  return (
    <Show when={any()} fallback={<Show when={empty()}>{(words) => <p class="pictures empty">{words()}</p>}</Show>}>
      <PicturesPanel run={props.run} state={props.state} viewer={props.viewer} />
    </Show>
  );
}

/** A run in one line: its job (or who ran it locally), how many checks ended, and a bar with a
 * cell per check in its state. The run shown lists its checks. */
function RunView(props: { run: Run; now: number; shown: boolean; onShow: () => void }) {
  const done = () => props.run.checks.filter((c) => c.entry).length;
  const by = () => localBy(props.run);
  // A leg's job is the same for every leg, so its name says the leg too, as GitHub's check does.
  const name = () => {
    const source = props.run.source;
    if (source.kind !== "ci") return "local";
    return source.leg && !source.job.includes(source.leg) ? `${source.job} ${source.leg}` : source.job;
  };
  // A local run proves nothing: it ran, it did not prove.
  const verb = () => {
    if (props.run.state === "cancelled") return "Stopped";
    if (by()) return props.run.state === "running" ? "Running" : "Ran";
    return props.run.state === "running" ? "Proving" : "Proved";
  };
  const about = () =>
    `${verb()} ${short(props.run.head)}${by() ? `, local, by ${by()}` : ""}, ${ago(props.now - props.run.opened)}`;
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
        <Checks checks={props.run.checks} now={props.now} />
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

/** The checks of the run shown: those that failed or still run each on a line, and the rest
 * folded by how they stand, since a list of passes says nothing a count does not. */
function Checks(props: { checks: RunCheck[]; now: number }) {
  // A passed check that passes only because it lists a flaky test stays in view, with that test.
  const shown = (c: RunCheck) => ["failed", "running"].includes(stateOf(c)) || listedFlaky(c).length > 0;
  const listed = () => props.checks.filter(shown);
  const folded = () =>
    (["passed", "not_run", "planned"] as const).flatMap((state) => {
      const checks = props.checks.filter((c) => stateOf(c) === state && !shown(c));
      return checks.length ? [{ state, checks }] : [];
    });
  return (
    <ul class="checks">
      <For each={listed()}>{(check) => <CheckView check={check} now={props.now} />}</For>
      <For each={folded()}>
        {(group) => (
          <li class="fold" data-fold={group.state}>
            <details>
              <summary>
                <StateIcon state={group.state} />
                <span>
                  <span class="num">{group.checks.length}</span> {WORDS[group.state]}
                </span>
              </summary>
              <ul class="checks">
                <For each={group.checks}>{(check) => <CheckView check={check} now={props.now} />}</For>
              </ul>
            </details>
          </li>
        )}
      </For>
    </ul>
  );
}

const STATE_ICONS: Record<CheckState, IconName> = {
  planned: "circle",
  running: "spinner",
  passed: "check",
  failed: "cross",
  not_run: "minus",
};

/** How a check stands, as an icon in its colour. */
function StateIcon(props: { state: CheckState }) {
  return (
    <span class={`si ${props.state}`} role="img" aria-label={WORDS[props.state]} title={WORDS[props.state]}>
      <Icon name={STATE_ICONS[props.state]} size={14} />
    </span>
  );
}

/** The tests a check passed with only because its `flaky_tests` lists them; a failed check's
 * tests show with why it failed. */
const listedFlaky = (check: RunCheck) =>
  check.entry?.outcome === "failed" ? [] : namedTests(check.entry).tests.flatMap((t) => (t.state === "listed" ? [t.name] : []));

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
      <StateIcon state={state()} />
      <span class="id">{props.check.id}</span>
      <span class="seconds">{seconds()}</span>
      <Show when={listedFlaky(props.check).length}>
        <ul class="tests">
          <For each={listedFlaky(props.check)}>
            {(name) => (
              <li data-test="listed" title="It failed, then passed on a rerun. The check lists it in flaky_tests, so it passes.">
                <span class="test-name">{name}</span>
                <span class="test-state">flaky, listed</span>
              </li>
            )}
          </For>
        </ul>
      </Show>
    </li>
  );
}
