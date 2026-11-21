// What the page sends the host: JSON from this page with its session, which every write needs.

/** What the host said to a write: its answer, or why it refused. */
export type Said<T> = { ok: true; answer: T } | { ok: false; error: string };

/** Sends `body` to `path` as JSON with the page's session. */
export const post = <T = unknown>(path: string, body: object) => send<T>("POST", path, body);

/** Sends `body` to `path` with `method` as JSON with the page's session. */
export async function send<T = unknown>(method: "POST" | "PATCH" | "DELETE", path: string, body: object): Promise<Said<T>> {
  try {
    const answer = await fetch(path, {
      method,
      credentials: "same-origin",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    });
    if (answer.ok) return { ok: true, answer: (await answer.json().catch(() => ({}))) as T };
    const refused = (await answer.json().catch(() => ({}))) as { error?: string };
    return { ok: false, error: refused.error ?? `the host answered ${answer.status}` };
  } catch {
    return { ok: false, error: "the host could not be reached; try again" };
  }
}

/** The error of a write the host refused, if it did. */
export const refusal = (said: Said<unknown>) => (said.ok ? undefined : said.error);
