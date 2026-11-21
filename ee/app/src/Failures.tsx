// What failed in the run shown, first on the page: each failed check with why it failed, what to
// do, the tests that failed, the findings on the lines the change touched, and the end of its
// output, folded. The runs column only says that it failed; this says why.

import { For, Show } from "solid-js";
import { Icon } from "./icons";
import { namedTests } from "./live";
import type { Check, Finding, Run } from "./proof";

/** The run's failed checks, in its order. */
export const failedOf = (run: Run): Check[] =>
  run.checks.flatMap((c) => (c.entry?.outcome === "failed" ? [c.entry] : []));

export function Failures(props: { checks: Check[] }) {
  return (
    <section class="failures" data-failures aria-labelledby="failures-title">
      <h2 id="failures-title" class="visually-hidden">
        Failed checks
      </h2>
      <For each={props.checks}>{(check) => <Failure check={check} />}</For>
    </section>
  );
}

/** The first letter up, as a sentence starts. */
const sentence = (text: string) => text.charAt(0).toUpperCase() + text.slice(1);

/** The last `n` lines of `text`. */
const tail = (text: string, n = 40) => text.trimEnd().split("\n").slice(-n).join("\n");

function Failure(props: { check: Check }) {
  const named = () => namedTests(props.check);
  const findings = () => (props.check.findings ?? []).filter((f) => f.level !== "hint");
  return (
    <article class="failure" data-failure={props.check.id}>
      <header>
        <span class="failure-icon">
          <Icon name="cross" size={14} />
        </span>
        <h3>
          <code>{props.check.id}</code>
        </h3>
        <span class="seconds num">{props.check.seconds.toFixed(1)} s</span>
      </header>
      <Show when={props.check.failure}>
        {(item) => (
          <>
            <p class="why" data-why>
              {sentence(item().why)}
            </p>
            <p class="fix" data-fix>
              {sentence(item().fix)}
            </p>
          </>
        )}
      </Show>
      <Show when={named().tests.length}>
        <ul class="failed-tests" aria-label="Tests">
          <For each={named().tests}>
            {(test) => (
              <li data-test={test.state}>
                <code>{test.name}</code>
                <Show when={test.state !== "failed"}>
                  <span class="tag">{test.state === "flaky" ? "passed only on a rerun" : "flaky, listed"}</span>
                </Show>
              </li>
            )}
          </For>
          <Show when={named().more}>{(more) => <li class="more">and {more()} more</li>}</Show>
        </ul>
      </Show>
      <Show when={findings().length}>
        <ul class="findings" aria-label="Findings">
          <For each={findings()}>{(f) => <FindingRow finding={f} />}</For>
        </ul>
      </Show>
      <Show when={props.check.output?.trim()}>
        {(output) => (
          <details class="output">
            <summary>Output</summary>
            <pre>{tail(output())}</pre>
          </details>
        )}
      </Show>
    </article>
  );
}

function FindingRow(props: { finding: Finding }) {
  const f = () => props.finding;
  const where = () => (f().anywhere ? f().file : `${f().file}:${f().line}`);
  return (
    <li data-finding={f().level}>
      <code class="where">{where()}</code>
      <span class="message">{f().message}</span>
      <code class="rule">{f().rule}</code>
    </li>
  );
}
