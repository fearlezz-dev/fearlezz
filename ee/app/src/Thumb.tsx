import { Show } from "solid-js";
import { fileUrl } from "./grant";
import { type Shown, isFilm, isText } from "./pictures";

/** A picture in the sheet, small: its after, or its before when it is gone; a film's last
 * frame, where it ends up (its first is often still blank); a text's summary. Empty until the files origin grants the run's files. */
export function Thumb(props: { shown: Shown; files: string }) {
  if (isText(props.shown.picture)) {
    return (
      <span class="text-thumb" data-text-thumb>
        {props.shown.picture.summary}
      </span>
    );
  }
  const path = () => props.shown.picture.after ?? props.shown.picture.before;
  const src = () => {
    const p = path();
    return p && props.files ? fileUrl(props.files, p) : "";
  };
  return (
    <Show when={src()} fallback={<span class="blank" />}>
      {(url) => (
        <Show when={isFilm(props.shown.picture)} fallback={<img src={url()} alt="" loading="lazy" decoding="async" draggable={false} />}>
          <video
            src={url()}
            muted
            playsinline
            preload="metadata"
            aria-hidden="true"
            onLoadedMetadata={(e) => {
              const v = e.currentTarget;
              if (Number.isFinite(v.duration)) v.currentTime = Math.max(0, v.duration - 0.05);
            }}
          />
        </Show>
      )}
    </Show>
  );
}
