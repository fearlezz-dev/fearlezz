// The few icons the pages draw, as inline SVG in the text's colour, so they follow the theme and
// need no requests. Each is 16 by 16 on a 24 grid, stroked, and hidden from screen readers:
// the words beside an icon say what it means.

import type { JSX } from "solid-js";

const PATHS = {
  check: "M5 12.5l4.5 4.5L19 7.5",
  cross: "M7 7l10 10M17 7L7 17",
  eye: "M2.5 12s3.5-6.5 9.5-6.5S21.5 12 21.5 12 18 18.5 12 18.5 2.5 12 2.5 12zM12 14.75a2.75 2.75 0 100-5.5 2.75 2.75 0 000 5.5z",
  branch: "M6 4v10M6 14a3 3 0 103 3M18 10a3 3 0 10-3-3M18 10c0 4-4 4-9 7",
  commit: "M12 15a3 3 0 100-6 3 3 0 000 6zM3 12h6M15 12h6",
  external: "M14 4h6v6M20 4l-9 9M18 14v5a1 1 0 01-1 1H5a1 1 0 01-1-1V7a1 1 0 011-1h5",
  pull: "M6 8a2.5 2.5 0 100-5 2.5 2.5 0 000 5zM6 8v13M18 21a2.5 2.5 0 100-5 2.5 2.5 0 000 5zM18 16V9a3 3 0 00-3-3h-4M13 3.5L10.5 6 13 8.5",
  shield: "M12 3l7.5 3v5.5c0 4.5-3.2 8.2-7.5 9.5-4.3-1.3-7.5-5-7.5-9.5V6L12 3zM9 12h6",
  stop: "M7 7h10v10H7z",
  circle: "M12 18a6 6 0 100-12 6 6 0 000 12z",
  minus: "M7 12h10",
  clock: "M12 21a9 9 0 100-18 9 9 0 000 18zM12 7v5l3 2",
  share: "M8.5 13.5l7-3.5M8.5 10.5l7 3.5M6 15a3 3 0 100-6 3 3 0 000 6zM18 9a3 3 0 100-6 3 3 0 000 6zM18 21a3 3 0 100-6 3 3 0 000 6z",
  expand: "M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5",
  shrink: "M9 4v5H4M15 4v5h5M9 20v-5H4M15 20v-5h5",
  chevron: "M7 10l5 5 5-5",
  alert: "M12 8v5M12 16.5v.5M10.3 3.9L2.6 17.5A2 2 0 004.3 20.5h15.4a2 2 0 001.7-3L13.7 3.9a2 2 0 00-3.4 0z",
} as const;

export type IconName = keyof typeof PATHS | "spinner";

export function Icon(props: { name: IconName; class?: string; size?: number }): JSX.Element {
  const size = () => props.size ?? 16;
  return (
    <svg
      class={`icon ${props.class ?? ""}`}
      data-icon={props.name}
      width={size()}
      height={size()}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      {props.name === "spinner" ? (
        <>
          <circle cx="12" cy="12" r="8.5" opacity="0.25" />
          <path d="M12 3.5a8.5 8.5 0 018.5 8.5" />
        </>
      ) : (
        <path d={PATHS[props.name]} />
      )}
    </svg>
  );
}

/** Fearlezz's mark: a proof's tick, cut out of a rounded square. */
export function Mark(): JSX.Element {
  return (
    <svg class="mark-logo" width="22" height="22" viewBox="0 0 24 24" aria-hidden="true">
      <rect width="24" height="24" rx="7" fill="currentColor" />
      <path d="M6.5 12.5l3.75 3.75L17.5 8.5" fill="none" stroke="var(--on-accent)" stroke-width="2.75" stroke-linecap="round" stroke-linejoin="round" />
    </svg>
  );
}
