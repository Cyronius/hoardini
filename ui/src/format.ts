const UNITS = ["B", "KB", "MB", "GB", "TB", "PB"];

export function humanBytes(n: number, digits = 1): string {
  if (!Number.isFinite(n)) return "-";
  const neg = n < 0;
  let v = Math.abs(n);
  let i = 0;
  while (v >= 1024 && i < UNITS.length - 1) {
    v /= 1024;
    i++;
  }
  const s = i === 0 ? String(Math.round(v)) : v.toFixed(v >= 100 ? 0 : digits);
  return `${neg ? "-" : ""}${s} ${UNITS[i]}`;
}

export function humanAge(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  if (s < 60) return "just now";
  const m = Math.floor(s / 60);
  if (m < 60) return `${m} min ago`;
  const h = Math.floor(m / 60);
  if (h < 48) return `${h} hour${h === 1 ? "" : "s"} ago`;
  const d = Math.floor(h / 24);
  return `${d} days ago`;
}

export function humanElapsed(ms: number): string {
  const s = ms / 1000;
  if (s < 60) return `${s.toFixed(1)}s`;
  const m = Math.floor(s / 60);
  return `${m}m ${Math.round(s - m * 60)}s`;
}

export function signedBytes(n: number): string {
  return `${n >= 0 ? "+" : "-"}${humanBytes(Math.abs(n))}`;
}

export function fileName(p: string): string {
  const parts = p.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? p;
}

export function friendlyToolName(name: string): string {
  return name.replace(/[_-]+/g, " ").trim();
}

export function shortDate(ms: number | null | undefined): string {
  if (!ms) return "";
  const d = new Date(ms);
  const thisYear = d.getFullYear() === new Date().getFullYear();
  return d.toLocaleDateString(undefined, thisYear ? { month: "short", day: "numeric" } : { month: "short", year: "numeric" });
}

// A time of day reads better than a date for a refresh earlier today.
export function sinceLabel(ms: number): string {
  const d = new Date(ms);
  return d.toDateString() === new Date().toDateString() ? d.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" }) : shortDate(ms);
}

export function samePath(a: string, b: string): boolean {
  return a.replace(/\\+$/, "").toLowerCase() === b.replace(/\\+$/, "").toLowerCase();
}

// An answer there would cover everything inside, so the server takes none for a drive, Users or a profile.
export function isRootFolder(path: string): boolean {
  const p = path.trim().replace(/\\+$/, "");
  return /^[A-Za-z]:$/.test(p) || p === "~" || /^[A-Za-z]:\\Users(\\[^\\]+)?$/i.test(p);
}

export function isUnder(parent: string, child: string): boolean {
  const p = parent.replace(/\\+$/, "").toLowerCase();
  const c = child.toLowerCase();
  return c.length > p.length + 1 && c.startsWith(`${p}\\`);
}

export function digits(n: number): string {
  return n.toLocaleString(undefined);
}
