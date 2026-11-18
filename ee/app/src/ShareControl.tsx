// The share control on a work's page, for a person with write access: make a link that shows
// anyone the work live, view only, see the live links, and revoke one.

import { For, Show, createSignal } from "solid-js";
import type { Share, Work } from "./proof";
import { listShares, makeShare, revokeShare } from "./share";
import { ago } from "./ago";

export function ShareControl(props: { work: Work }) {
  const [open, setOpen] = createSignal(false);
  const [shares, setShares] = createSignal<Share[]>([]);
  const [made, setMade] = createSignal("");
  const [copied, setCopied] = createSignal(false);
  const [error, setError] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const load = async () => {
    const listed = await listShares(props.work);
    if ("error" in listed) setError(listed.error);
    else setShares(listed.shares);
  };
  const toggle = () => {
    const next = !open();
    setOpen(next);
    setError("");
    if (next) void load();
    else setMade("");
  };
  const make = async () => {
    setBusy(true);
    const answer = await makeShare(props.work);
    setBusy(false);
    if ("error" in answer) {
      setError(answer.error);
      return;
    }
    setError("");
    setCopied(false);
    setMade(answer.url);
    await load();
  };
  const revoke = async (id: string) => {
    const refused = await revokeShare(id);
    setError(refused ?? "");
    await load();
  };
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(made());
      setCopied(true);
    } catch {
      document.querySelector<HTMLInputElement>("[data-made] input")?.select();
    }
  };
  return (
    <div class="share">
      <button type="button" class="plain" data-share aria-expanded={open()} onClick={toggle}>
        Share
      </button>
      <Show when={open()}>
        <section class="share-panel" data-shares aria-label="Share links">
          <p class="hint">Anyone with a link sees this work live, view only, until someone revokes it.</p>
          <Show
            when={made()}
            fallback={
              <button type="button" class="btn" data-make-share disabled={busy()} onClick={() => void make()}>
                Make a link
              </button>
            }
          >
            <div class="made" data-made>
              <input readonly value={made()} aria-label="The new link" onFocus={(e) => e.currentTarget.select()} />
              <button type="button" class="btn" onClick={() => void copy()}>
                {copied() ? "Copied" : "Copy"}
              </button>
            </div>
            <p class="hint">Copy it now: it is shown only once.</p>
          </Show>
          <Show when={shares().length}>
            <ul class="share-list">
              <For each={shares()}>
                {(share) => (
                  <li data-share-id={share.id}>
                    <span>
                      Made by <b>{share.made_by}</b>, {ago(Date.now() / 1000 - share.made_at)}
                    </span>
                    <button type="button" class="plain" data-revoke onClick={() => void revoke(share.id)}>
                      Revoke
                    </button>
                  </li>
                )}
              </For>
            </ul>
          </Show>
          <Show when={error()}>
            <p class="error" role="alert">
              {error()}
            </p>
          </Show>
        </section>
      </Show>
    </div>
  );
}
