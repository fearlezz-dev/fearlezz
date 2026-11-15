// A repository's page: every work of it, newest activity first, each with where its newest run
// stands and what waits on it, live. Each row leads to the work's page.

import { For, Match, Show, Switch, createSignal, onCleanup } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import { ago } from "./ago";
import { type Status, connect } from "./live";
import type { Listed } from "./proof";
import { ListingFeed, type Standing, type Works, standingOf, waitsOn } from "./repository";

const WORDS: Record<Standing, string> = {
  none: "no run yet",
  running: "running",
  passed: "passed",
  failed: "failed",
  cancelled: "stopped",
};

export function RepositoryPage(props: { repository: string }) {
  const [status, setStatus] = createSignal<Status["kind"]>("connecting");
  const [gone, setGone] = createSignal("");
  const [list, setList] = createStore<{ state?: Works }>({});
  const [now, setNow] = createSignal(Date.now() / 1000);
  const tick = setInterval(() => setNow(Date.now() / 1000), 30_000);
  const stop = connect(
    location.pathname,
    () => new ListingFeed(),
    (next: Status<Works>) => {
      setStatus(next.kind);
      if (next.kind === "live") setList("state", reconcile(next.state, { key: "path" }));
      if (next.kind === "gone") setGone(next.why);
    },
  );
  onCleanup(() => {
    stop();
    clearInterval(tick);
  });
  document.title = `${props.repository} · Fearlezz`;
  return (
    <main class="repository">
      <header class="top">
        <div class="crumb">
          <span class={`live ${status()}`} title={status() === "live" ? "Updates live" : "Reconnecting"}>
            {status() === "live" ? "live" : "reconnecting"}
          </span>
        </div>
        <h1>{list.state?.repository ?? props.repository}</h1>
      </header>
      <Switch>
        <Match when={status() === "gone"}>
          <p class="note">{gone()}</p>
        </Match>
        <Match when={list.state}>
          {(state) => (
            <ol class="works" aria-label="Works, newest activity first">
              <For each={state().works}>{(listed) => <Row listed={listed} now={now()} />}</For>
            </ol>
          )}
        </Match>
        <Match when={true}>
          <p class="note">Connecting…</p>
        </Match>
      </Switch>
    </main>
  );
}

function Row(props: { listed: Listed; now: number }) {
  const standing = () => standingOf(props.listed);
  const pull = () => props.listed.work.pull;
  const local = () => (props.listed.latest?.source.kind === "local" ? props.listed.latest.source.login || "a developer" : "");
  const waits = () => waitsOn(props.listed);
  return (
    <li class="work-row" data-listed={props.listed.path} data-state={standing()}>
      <a href={props.listed.path} title={`${WORDS[standing()]}, ${ago(props.now - props.listed.active)}`}>
        <span class={`dot ${standing()}`} role="img" aria-label={WORDS[standing()]} />
        <span class="what">
          <span class="title">
            <Show when={pull()} fallback={<span class="branch">{props.listed.work.branch}</span>}>
              {(p) => (
                <>
                  <span class="number">#{p().number}</span> {p().title}
                </>
              )}
            </Show>
          </span>
          <Show when={pull()}>
            <span class="meta branch">{props.listed.work.branch}</span>
          </Show>
        </span>
        <span class="waits">
          <Show when={local()}>
            <span class="tag unproven" data-unproven title="The newest run is a local run: the agent's word, never reviewed">
              local, {local()}
            </span>
          </Show>
          <Show when={props.listed.latest && props.listed.latest.failed > 0 && standing() !== "failed"}>
            <span class="wait bad">{props.listed.latest!.failed} failed so far</span>
          </Show>
          <For each={waits()}>{(w) => <span class="wait">{w}</span>}</For>
        </span>
      </a>
    </li>
  );
}
