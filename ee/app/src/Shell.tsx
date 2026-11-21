// What every page of the host shares: the bar on top (Fearlezz, where the page is, whether it
// is live, and the page's own controls), and the placeholder a page shows while it connects.

import { For, type JSX, Show } from "solid-js";
import { Mark } from "./icons";
import type { Status } from "./live";

/** One step of where a page is: a link, or the page itself. */
export type Crumb = { label: string; href?: string | undefined; mono?: boolean; data?: string };

export function TopBar(props: { crumbs: Crumb[]; status: Status["kind"]; children?: JSX.Element }) {
  return (
    <header class="appbar">
      <div class="appbar-in">
        <span class="brand">
          <Mark />
          <span class="brand-name">Fearlezz</span>
        </span>
        <nav class="crumbs" aria-label="Where this page is">
          <For each={props.crumbs}>
            {(crumb) => (
              <>
                <span class="sep" aria-hidden="true">
                  /
                </span>
                <Show when={crumb.href} fallback={<span class={crumb.mono ? "crumb mono" : "crumb"}>{crumb.label}</span>}>
                  {(href) => (
                    <a class={crumb.mono ? "crumb mono" : "crumb"} href={href()} {...(crumb.data ? { [`data-${crumb.data}`]: "" } : {})}>
                      {crumb.label}
                    </a>
                  )}
                </Show>
              </>
            )}
          </For>
        </nav>
        <div class="appbar-end">
          <span class={`live is-${props.status}`} title={props.status === "live" ? "Updates live" : "Reconnecting"}>
            {props.status === "live" ? "Live" : props.status === "gone" ? "Offline" : "Connecting"}
          </span>
          {props.children}
        </div>
      </div>
    </header>
  );
}

/** The shape of a page while its live connection opens, so nothing jumps when it fills. */
export function Placeholder(props: { rows?: number }) {
  return (
    <div class="placeholder" aria-busy="true" aria-label="Connecting">
      <span class="ph title" />
      <span class="ph meta" />
      <span class="ph strip" />
      <div class="ph-cols">
        <div class="ph-main">
          <For each={Array.from({ length: props.rows ?? 3 })}>{() => <span class="ph block" />}</For>
        </div>
        <span class="ph side" />
      </div>
    </div>
  );
}
