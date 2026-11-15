// Share links of a work, for a person with write access: make one, see the live ones, revoke
// one. The host keeps only a token's HMAC, so a link is shown once, when it is made.

import { post, refusal } from "./api";
import type { Work } from "./proof";

/** A live link, as the list shows it: who made it and when, never the link itself. */
export type Share = { id: string; made_by: string; made_at: number };

const query = (work: Work) =>
  `repository=${encodeURIComponent(work.repository)}&branch=${encodeURIComponent(work.branch)}`;

/** The work's live links, newest first, or why the host would not say. */
export async function listShares(work: Work): Promise<{ shares: Share[] } | { error: string }> {
  try {
    const answer = await fetch(`/api/v2/shares?${query(work)}`, { credentials: "same-origin" });
    const body = (await answer.json().catch(() => ({}))) as { shares?: Share[]; error?: string };
    if (answer.ok && body.shares) return { shares: body.shares };
    return { error: body.error ?? `the host answered ${answer.status}` };
  } catch {
    return { error: "the host could not be reached; try again" };
  }
}

/** Makes a link to the work: its URL, shown only now, or why the host refused. */
export async function makeShare(work: Work): Promise<{ url: string } | { error: string }> {
  const said = await post<{ url: string }>("/api/v2/shares", { repository: work.repository, branch: work.branch });
  return said.ok ? { url: said.answer.url } : { error: said.error };
}

/** Revokes link `id`. The error the host gave, if it refused. */
export const revokeShare = async (id: string) => refusal(await post(`/api/v2/shares/${encodeURIComponent(id)}/revoke`, {}));
