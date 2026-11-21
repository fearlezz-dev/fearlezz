// A WebM (Matroska) film's video frames as WebCodecs decodes them, with each frame's start and
// duration as the CLI reads them from ffprobe, so the page's comparison gets the same timing.
// The page compares films the checks recorded, which are WebM; anything else it leaves to CI.

/** One frame of the film: its bytes as stored, whether it decodes alone, and its time. */
export type Block = { data: Uint8Array; key: boolean; start: number; dur: number };

/** A film's codec (Matroska's id), size and frames in the order they are shown. */
export type Demuxed = { codec: string; w: number; h: number; blocks: Block[] };

// Master elements are entered rather than skipped, so a live recording's clusters of unknown
// size need no special case: EBML header and Segment, Info, Tracks, TrackEntry, Video,
// Cluster and BlockGroup.
const MASTERS = new Set([0x18538067, 0x1549a966, 0x1654ae6b, 0xae, 0xe0, 0x1f43b675, 0xa0]);

/** A variable length number at `at`: its value (with its length marker when `marked`, as an
 * element id keeps it) and its length in bytes. */
function vint(b: Uint8Array, at: number, marked: boolean): { v: number; len: number } {
  const first = b[at] ?? 0;
  let len = 1;
  let mask = 0x80;
  while (len <= 8 && !(first & mask)) {
    len++;
    mask >>= 1;
  }
  let v = marked ? first : first & (mask - 1);
  for (let i = 1; i < len; i++) v = v * 256 + (b[at + i] ?? 0);
  return { v, len };
}

function uint(b: Uint8Array, at: number, n: number): number {
  let v = 0;
  for (let i = 0; i < n; i++) v = v * 256 + (b[at + i] ?? 0);
  return v;
}

/** Seconds as ffprobe prints them (six decimals) and the CLI parses them back. */
const printed = (s: number) => Number(s.toFixed(6));

type Track = { number: number; type: number; codec: string; w: number; h: number; defaultDuration: number };
type Raw = { track: number; ticks: number; dur: number | null; key: boolean; data: Uint8Array };

/** The video frames of the WebM film in `buf`. Times are as the CLI's probe takes them: each
 * frame's own duration (or its track's default, in whole ticks), else until the next frame, else
 * 0.05 s, sorted by start.
 *
 * Throws on a film that is not WebM or has no video, and on laced blocks, which recordings
 * never hold. */
export function demux(buf: ArrayBuffer): Demuxed {
  const b = new Uint8Array(buf);
  let scale = 1_000_000;
  let cluster = 0;
  const tracks: Track[] = [];
  const raw: Raw[] = [];
  let p = 0;
  while (p < b.length) {
    const id = vint(b, p, true);
    p += id.len;
    const size = vint(b, p, false);
    p += size.len;
    if (id.v === 0xae) tracks.push({ number: 0, type: 0, codec: "", w: 0, h: 0, defaultDuration: 0 });
    if (MASTERS.has(id.v)) continue;
    const at = p;
    p += size.v;
    const track = tracks[tracks.length - 1];
    switch (id.v) {
      case 0x2ad7b1:
        scale = uint(b, at, size.v);
        break;
      case 0xd7:
        if (track) track.number = uint(b, at, size.v);
        break;
      case 0x83:
        if (track) track.type = uint(b, at, size.v);
        break;
      case 0x86:
        if (track) track.codec = new TextDecoder().decode(b.subarray(at, at + size.v));
        break;
      case 0xb0:
        if (track) track.w = uint(b, at, size.v);
        break;
      case 0xba:
        if (track) track.h = uint(b, at, size.v);
        break;
      case 0x23e383:
        if (track) track.defaultDuration = uint(b, at, size.v);
        break;
      case 0xe7:
        cluster = uint(b, at, size.v);
        break;
      // BlockDuration and ReferenceBlock belong to the BlockGroup whose Block came last.
      case 0x9b: {
        const last = raw[raw.length - 1];
        if (last) last.dur = uint(b, at, size.v);
        break;
      }
      case 0xfb: {
        const last = raw[raw.length - 1];
        if (last) last.key = false;
        break;
      }
      case 0xa3:
      case 0xa1: {
        const number = vint(b, at, false);
        const relative = (((b[at + number.len] ?? 0) << 24) >> 16) | (b[at + number.len + 1] ?? 0);
        const flags = b[at + number.len + 2] ?? 0;
        if (flags & 0x06) throw new Error("the film has laced blocks, which the page cannot read");
        raw.push({
          track: number.v,
          ticks: cluster + relative,
          dur: null,
          // A SimpleBlock says whether it is a key frame; a Block is one unless it refers back.
          key: id.v === 0xa3 ? !!(flags & 0x80) : true,
          data: b.subarray(at + number.len + 3, at + size.v),
        });
        break;
      }
    }
  }
  const video = tracks.find((t) => t.type === 1);
  if (!video) throw new Error("the film has no video the page can read");
  const own = raw.filter((r) => r.track === video.number);
  const timed = own.map((r) => ({
    r,
    start: printed((r.ticks * scale) / 1e9),
    // ffprobe prints a duration of 0 as missing.
    dur:
      r.dur != null && r.dur > 0
        ? printed((r.dur * scale) / 1e9)
        : video.defaultDuration >= scale
          ? printed((Math.floor(video.defaultDuration / scale) * scale) / 1e9)
          : null,
  }));
  timed.sort((x, y) => x.start - y.start);
  const blocks = timed.map((k, i) => {
    const next = timed[i + 1];
    return { data: k.r.data, key: k.r.key, start: k.start, dur: k.dur ?? (next ? next.start - k.start : 0.05) };
  });
  return { codec: video.codec, w: video.w, h: video.h, blocks };
}

/** The WebCodecs codec string for a Matroska codec id. */
export function codecOf(codec: string): string {
  switch (codec) {
    case "V_VP9":
      return "vp09.00.10.08";
    case "V_VP8":
      return "vp8";
    case "V_AV1":
      return "av01.0.08M.08";
    default:
      throw new Error(`the page cannot decode ${codec || "this codec"}`);
  }
}
