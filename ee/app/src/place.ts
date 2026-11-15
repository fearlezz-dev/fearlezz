// Which page the app is on, from its path: a repository's list of works, a work's page, or a
// work's page through a share link, which only views.

export type Place = { kind: "repository"; repository: string } | { kind: "share"; token: string } | { kind: "work" };

export function placeOf(path: string): Place {
  const parts = path.split("/").filter((p) => p !== "");
  // A GitHub owner never starts with a dash, so `/-/` names no repository.
  if (parts.length === 3 && parts[0] === "-" && parts[1] === "share") return { kind: "share", token: parts[2]! };
  if (parts.length === 2) return { kind: "repository", repository: parts.map(decodeURIComponent).join("/") };
  return { kind: "work" };
}

/** The token of the share link this page is opened through, if any. */
export function sharedBy(path: string): string | undefined {
  const place = placeOf(path);
  return place.kind === "share" ? place.token : undefined;
}
