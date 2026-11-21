// A repository's page: every work of it, newest activity first, each with where its newest run
// stands and what waits on it, live. Each row leads to the work's page.

import { For, Match, Show, Switch, createSignal, onCleanup } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import { ago } from "./ago";
import { Icon } from "./icons";
import { TopBar } from "./Shell";
import { type Status, connect } from "./live";
import type { Listed } from "./proof";
import { ListingFeed, type Works } from "./repository";

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
  const count = () => list.state?.works.length ?? 0;
  return (
    <>
      <TopBar crumbs={[{ label: list.state?.repository ?? props.repository }]} status={status()} />
      <main class="repository">
        <header class="hero">
          <h1>{list.state?.repository ?? props.repository}</h1>
          <Show when={list.state}>
            <p class="meta">
              {count()} {count() === 1 ? "work" : "works"}
            </p>
          </Show>
        </header>
        <Switch>
          <Match when={status() === "gone"}>
            <div class="empty">
              <Icon name="alert" size={22} />
              <p class="note">{gone()}</p>
            </div>
          </Match>
          <Match when={list.state?.works.length === 0}>
            <div class="empty">
              <Icon name="clock" size={22} />
              <p class="note">No works yet. A branch shows here once CI proves it.</p>
            </div>
          </Match>
          <Match when={list.state}>
            {(state) => (
              <ol class="works" aria-label="Works, newest activity first">
                <For each={state().works}>{(listed) => <Row listed={listed} now={now()} />}</For>
              </ol>
            )}
          </Match>
          <Match when={true}>
            <div class="placeholder" aria-busy="true" aria-label="Connecting">
              <For each={[0, 1, 2, 3]}>{() => <span class="ph row" />}</For>
            </div>
          </Match>
        </Switch>
      </main>
    </>
  );
}

function Row(props: { listed: Listed; now: number }) {
  const standing = () => props.listed.standing;
  const pull = () => props.listed.work.pull;
  const local = () => (props.listed.latest?.source.kind === "local" ? props.listed.latest.source.login || "a developer" : "");
  return (
    <li class="work-row" data-listed={props.listed.path} data-state={standing()}>
      <a href={props.listed.path}>
        <span class={`dot ${standing()}`} role="img" aria-label={props.listed.word} />
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
          <span class="sub">
            <Show when={pull()}>
              <span class="meta branch">{props.listed.work.branch}</span>
            </Show>
            <Show when={local()}>
              <span class="tag unproven" data-unproven title="The newest run is a local run: the agent's word, never reviewed">
                local, {local()}
              </span>
            </Show>
            <Show when={props.listed.latest && props.listed.latest.failed > 0 && standing() !== "failed"}>
              <span class="wait bad">{props.listed.latest!.failed} failed so far</span>
            </Show>
            <For each={props.listed.waits}>{(w) => <span class="wait">{w}</span>}</For>
          </span>
        </span>
        <span class="when">
          <span class={`word ${standing()}`}>{props.listed.word}</span>
          <span class="ago">{ago(props.now - props.listed.active)}</span>
        </span>
      </a>
    </li>
  );
}
