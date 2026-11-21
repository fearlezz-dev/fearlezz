// The pictures a run in these states took: small drawn screens of a made up app, as SVG, so the
// fixtures need no binary files. Each takes the parts that differ between a before and after.

type Screen = { accent: string; rows: string[]; banner?: string; dark?: boolean };

const W = 1280;
const H = 800;

/** A settings screen of a made up app, `rows` as its fields. */
export function screen({ accent, rows, banner, dark }: Screen): string {
  const bg = dark ? "#16181d" : "#f6f7f9";
  const card = dark ? "#1f2229" : "#ffffff";
  const ink = dark ? "#e7e9ee" : "#1b1e24";
  const faint = dark ? "#3a3f4a" : "#e2e5ea";
  const field = rows
    .map(
      (label, i) => `
    <text x="360" y="${290 + i * 84}" font-size="15" fill="${ink}" opacity=".7">${label}</text>
    <rect x="360" y="${302 + i * 84}" width="560" height="40" rx="8" fill="none" stroke="${faint}" stroke-width="2"/>`,
    )
    .join("");
  const top = 302 + rows.length * 84;
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}" font-family="system-ui, sans-serif">
  <rect width="${W}" height="${H}" fill="${bg}"/>
  <rect width="${W}" height="64" fill="${card}"/>
  <circle cx="44" cy="32" r="14" fill="${accent}"/>
  <text x="72" y="38" font-size="18" font-weight="600" fill="${ink}">Harbor</text>
  <rect x="0" y="64" width="240" height="${H - 64}" fill="${card}" opacity=".6"/>
  ${["Profile", "Team", "Billing", "Security"].map((t, i) => `<text x="32" y="${120 + i * 44}" font-size="15" fill="${ink}" opacity="${i === 0 ? 1 : 0.55}">${t}</text>`).join("")}
  <rect x="320" y="${banner ? 180 : 140}" width="640" height="${top - (banner ? 120 : 80)}" rx="14" fill="${card}"/>
  ${banner ? `<rect x="320" y="104" width="640" height="56" rx="10" fill="${accent}" opacity=".14"/><text x="344" y="138" font-size="15" fill="${ink}">${banner}</text>` : ""}
  <text x="360" y="${banner ? 236 : 196}" font-size="24" font-weight="650" fill="${ink}">Profile</text>
  ${field}
  <rect x="360" y="${top + 8}" width="132" height="42" rx="8" fill="${accent}"/>
  <text x="426" y="${top + 35}" font-size="15" font-weight="600" fill="#fff" text-anchor="middle">Save</text>
</svg>`;
}

export const SVG = "image/svg+xml";
export const SIZE: [number, number] = [W, H];
