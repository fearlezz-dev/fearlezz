import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { codecOf, demux } from "./demux";

const film = (path: string) => {
  const bytes = readFileSync(new URL(path, import.meta.url));
  return demux(bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength));
};

describe("demux", () => {
  // ffprobe gives these films' packets as 0.1 s (and 0.04 s) apart, each lasting that long,
  // the first a key frame: the times the CLI compares them by.
  test("reads a VP9 film's frames with the times ffprobe gives", () => {
    const f = film("../../../crates/fearlezz/tests/fixtures/compare/now.webm");
    expect([f.codec, f.w, f.h]).toEqual(["V_VP9", 640, 400]);
    expect(f.blocks.length).toBe(20);
    expect(f.blocks.map((b) => b.start)).toEqual(Array.from({ length: 20 }, (_, i) => Number((i / 10).toFixed(6))));
    expect(f.blocks.every((b) => b.dur === 0.1)).toBe(true);
    expect(f.blocks.map((b) => b.key)).toEqual([true, ...Array(19).fill(false)]);
  });

  test("reads a recording of another rate", () => {
    const f = film("../e2e/fixtures/checkout.webm");
    expect(f.blocks.length).toBe(50);
    expect(f.blocks[49]!.start).toBe(1.96);
    expect(f.blocks.every((b) => b.dur === 0.04)).toBe(true);
  });

  test("refuses what is not a film", () => {
    expect(() => demux(new Uint8Array([1, 2, 3]).buffer)).toThrow("no video");
  });

  test("names the WebCodecs codec, or says the page cannot decode it", () => {
    expect(codecOf("V_VP9")).toBe("vp09.00.10.08");
    expect(() => codecOf("V_MPEG4/ISO/AVC")).toThrow("cannot decode V_MPEG4/ISO/AVC");
  });
  // A film comes from a pull request, so its sizes are not trusted: a number element claiming
  // terabytes is refused at once rather than looped over.
  test("refuses a number element larger than 8 bytes", () => {
    const hostile = new Uint8Array([0x2a, 0xd7, 0xb1, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00]).buffer;
    expect(() => demux(hostile)).toThrow("not WebM the page can read");
  });
});
