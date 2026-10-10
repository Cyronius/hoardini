import { useEffect, useRef, useState, type ReactNode } from "react";
import { routes } from "../api/client";
import type { LocationAnswer, ReviewArea } from "../api/types";
import { ItemTree, RevealButton, Toggle, actionBtn, overlay } from "../components/ItemTree";
import { askAbout, cleanUpTitle } from "../components/ToolCards";
import { digits, fileName, humanBytes } from "../format";
import { useStore } from "../store";
import { useReview } from "./state";

const ANSWERS: { a: LocationAnswer; label: string; title: string }[] = [
  { a: "keep", label: "Keep", title: "It's yours: Hoardini never proposes deleting it, but may offer to move or compress it" },
  { a: "disposable", label: "Disposable", title: "It can go: the next review may propose deleting it" },
  { a: "move-elsewhere", label: "Move elsewhere", title: "Keep it, but on another drive: the next review may propose sending it to a destination" },
];

const lookCloserText = (path: string) => `What is ${path}? Look inside and tell me whether to keep it, delete it, or move it elsewhere.`;

// Areas carry no kind; a name with a known file extension has nothing inside to list.
const FILE_RE = /\.(sys|vhdx?|vmdk|vdi|qcow2|img|iso|dmp|gguf|safetensors|onnx|bin|pt|ckpt|q4nx|zip|7z|rar|tar|gz|zst|cab|wim|esd|msi|exe|pack|pst|ost|mp4|mkv|mov|db|sqlite|log|etl)$/i;
export const looksLikeFile = (path: string) => FILE_RE.test(fileName(path));

const pill =
  "rounded-full border border-line bg-ink-900 px-2.5 py-0.5 text-[length:var(--fs-xs)] font-bold text-paper-dim hover:border-brass/60 hover:text-brass focus-visible:outline focus-visible:outline-1 focus-visible:outline-brass disabled:opacity-40";

export function WhatIsButtons({ path, onAnswered }: { path: string; onAnswered?: (text: string, a: LocationAnswer) => void }) {
  const answer = useReview((s) => s.answer);
  const askChat = useStore((s) => s.askChat);
  const busyChat = useStore((s) => s.chatBusy);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [menu, setMenu] = useState(false);
  const more = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!menu) return;
    const down = (e: MouseEvent) => !more.current?.contains(e.target as Node) && setMenu(false);
    const key = (e: KeyboardEvent) => e.key === "Escape" && setMenu(false);
    document.addEventListener("mousedown", down);
    document.addEventListener("keydown", key);
    return () => {
      document.removeEventListener("mousedown", down);
      document.removeEventListener("keydown", key);
    };
  }, [menu]);

  const say = async (a: LocationAnswer) => {
    setBusy(true);
    setErr(null);
    setMenu(false);
    try {
      const m = await answer(path, a);
      onAnswered?.(m.text, a);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex flex-wrap items-center gap-1.5" role="group" aria-label={`What is ${fileName(path)}?`}>
      <span className="mr-0.5 text-[length:var(--fs-xs)] font-bold text-muted">What is this?</span>
      {ANSWERS.map((x) => (
        <button key={x.a} type="button" className={pill} disabled={busy} title={x.title} onClick={() => void say(x.a)}>
          {x.label}
        </button>
      ))}
      <button
        type="button"
        className={`${pill} border-brass/40 text-brass`}
        disabled={busyChat}
        title="Hoardini looks inside with the AI and answers in the chat"
        onClick={() => askChat(lookCloserText(path), path)}
      >
        Look closer
      </button>
      <div ref={more} className="relative">
        <button type="button" className={`${pill} px-2`} aria-haspopup="menu" aria-expanded={menu} aria-label="More answers" title="More answers" disabled={busy} onClick={() => setMenu((m) => !m)}>
          ⋯
        </button>
        {menu && (
          <div role="menu" className="fade-up absolute right-0 top-full z-20 mt-1 w-60 rounded-xl border border-line bg-ink-850 p-1 shadow-lg">
            <button
              type="button"
              role="menuitem"
              autoFocus
              className="block w-full rounded-lg px-2.5 py-1.5 text-left hover:bg-ink-800 focus-visible:bg-ink-800 focus-visible:outline-none"
              onClick={() => void say("never-touch")}
            >
              <span className="block text-[length:var(--fs-sm)] font-bold text-paper">Never touch</span>
              <span className="block text-[length:var(--fs-xs)] text-muted">Hoardini never proposes anything in it, not even a move.</span>
            </button>
          </div>
        )}
      </div>
      {err && <span className="text-[length:var(--fs-xs)] font-semibold text-err">{err}</span>}
    </div>
  );
}

function AreaRow({ area, max }: { area: ReviewArea; max: number }) {
  const [open, setOpen] = useState(false);
  const askChat = useStore((s) => s.askChat);
  const cleanUp = useReview((s) => s.cleanUp);
  const name = fileName(area.path) || area.path;
  const file = looksLikeFile(area.path);
  const parent = area.path.slice(0, area.path.length - name.length).replace(/\\$/, "");
  const known = area.label && area.label !== "unknown";
  // A user's Disposable makes the folder a candidate; catalog rows with every action are still not proposed here.
  const canClean = !file && area.verdict === "safe" && area.actions.includes("delete");
  return (
    <li className={`relative py-2 pl-3 ${area.unknown ? "area-unknown" : ""}`} data-area={area.path}>
      <div className="group flex items-start gap-2">
        <span className="mt-0.5 w-4 shrink-0">{!file && <Toggle open={open} onClick={() => setOpen((o) => !o)} label={name} />}</span>
        <div className="min-w-0 flex-1">
          <div className="flex items-baseline gap-2">
            <span className="min-w-0 shrink-0 truncate font-bold text-paper" title={area.path}>
              {name}
            </span>
            {known ? (
              <span className="hidden max-w-[45%] shrink-0 truncate rounded-full border border-line px-2 py-px text-[length:var(--fs-xs)] font-semibold text-muted @md:inline" title={area.label}>
                {area.label}
              </span>
            ) : (
              area.unknown && <span className="shrink-0 rounded-full border border-brass/40 px-2 py-px text-[length:var(--fs-xs)] font-bold text-brass">unknown</span>
            )}
            <span className="min-w-0 flex-1 truncate text-[length:var(--fs-xs)] text-faint" title={area.path}>
              {parent}
            </span>
            <span className="shrink-0 font-bold tabular-nums text-paper">{humanBytes(area.size)}</span>
          </div>
          <div className="relative mt-1 flex h-5 items-center">
            <span className="block h-1 w-full overflow-hidden rounded-full bg-ink-800" aria-hidden="true">
              <span className={`block h-full rounded-full ${area.unknown ? "bg-brass/60" : "bg-muted/50"}`} style={{ width: `${Math.max(1, (area.size / max) * 100)}%` }} />
            </span>
            <span className={`${overlay} bg-ink-850`}>
              <button type="button" className={actionBtn} title={askAbout(area.path)} onClick={() => askChat(askAbout(area.path))}>
                Ask
              </button>
              {canClean && (
                <button type="button" className={actionBtn} title={cleanUpTitle(area.path)} onClick={() => void cleanUp(area.path)}>
                  Clean up
                </button>
              )}
              <RevealButton path={area.path} />
            </span>
          </div>
          <p className="text-[length:var(--fs-sm)] text-paper-dim">
            {area.why_not}
            {area.hint && <span className="ml-1.5 font-semibold text-info">→ {area.hint}</span>}
          </p>
          {area.unknown && (
            <div className="mt-1.5">
              <WhatIsButtons path={area.path} />
            </div>
          )}
        </div>
      </div>
      {open && (
        <div className="ml-6">
          <ItemTree path={area.path} rootSize={area.size} />
        </div>
      )}
    </li>
  );
}

const SHOWN = 10;

export function LargeAreas({ areas, scope }: { areas: ReviewArea[]; scope: string }) {
  const [all, setAll] = useState(false);
  if (areas.length === 0) return null;
  const max = Math.max(1, ...areas.map((a) => a.size));
  const unknown = areas.filter((a) => a.unknown).length;
  const shown = all ? areas : areas.slice(0, SHOWN);
  const rest = areas.slice(SHOWN);
  return (
    <section aria-label="Large areas" className="@container">
      <SectionHead title="Large areas" count={areas.length}>
        Big things in {scope} that Hoardini does not propose, and why.
        {unknown > 0 && ` ${unknown} ${unknown === 1 ? "is" : "are"} not identified yet: say what ${unknown === 1 ? "it is" : "they are"}.`}
      </SectionHead>
      <ul className="divide-y divide-line-soft rounded-2xl border border-line bg-ink-850 px-3">
        {shown.map((a) => (
          <AreaRow key={a.path} area={a} max={max} />
        ))}
        {!all && rest.length > 0 && (
          <li className="py-2 text-center">
            <button type="button" className={actionBtn} onClick={() => setAll(true)}>
              Show all {areas.length} · {rest.length} more, {humanBytes(rest.reduce((a, x) => a + x.size, 0))}
              {rest.some((a) => a.unknown) ? `, ${rest.filter((a) => a.unknown).length} not identified` : ""}
            </button>
          </li>
        )}
      </ul>
    </section>
  );
}

export function SectionHead({ title, count, children }: { title: string; count?: number; children?: ReactNode }) {
  return (
    <header className="mb-2 mt-1">
      <h2 className="flex items-baseline gap-2 font-[family-name:var(--font-display)] text-[17px] font-bold text-paper">
        {title}
        {count != null && <span className="text-[length:var(--fs-sm)] font-semibold tabular-nums text-faint">{digits(count)}</span>}
      </h2>
      {children && <p className="text-[length:var(--fs-sm)] text-muted">{children}</p>}
    </header>
  );
}

// From Explore's What is this? or a `what_is` pill: the same answers for any path, at the top of the review.
export function WhatIsCard({ path }: { path: string }) {
  const showWhatIs = useReview((s) => s.showWhatIs);
  const area = useReview((s) => [...(s.folder?.view?.areas ?? []), ...(s.main?.areas ?? [])].find((a) => a.path.toLowerCase() === path.toLowerCase()));
  const [info, setInfo] = useState<{ size: number; files: number } | null>(null);
  const [saved, setSaved] = useState<{ text: string; a: LocationAnswer } | null>(null);
  const cleanUp = useReview((s) => s.cleanUp);
  const [open, setOpen] = useState(false);
  const file = looksLikeFile(path);
  const name = fileName(path) || path;

  useEffect(() => {
    setSaved(null);
    setInfo(null);
    if (file) return;
    let live = true;
    routes
      .children(path, 1)
      .then((c) => live && setInfo({ size: c.size, files: c.files }))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [path, file]);

  const size = area?.size ?? info?.size;
  return (
    <section aria-label="What is this?" className="fade-up @container rounded-2xl border border-brass/40 bg-ink-850 p-4 shadow-sm">
      <div className="flex items-start gap-2">
        <div className="min-w-0 flex-1">
          <div className="label text-[length:var(--fs-xs)] uppercase">What is this?</div>
          <div className="mt-0.5 flex items-baseline gap-2">
            <span className="min-w-0 truncate font-[family-name:var(--font-display)] text-[length:var(--fs-md)] font-bold text-paper" title={path}>
              {name}
            </span>
            {size != null && <span className="shrink-0 font-bold tabular-nums text-paper">{humanBytes(size)}</span>}
            {info && <span className="hidden shrink-0 text-[length:var(--fs-xs)] tabular-nums text-faint @sm:inline">{digits(info.files)} files</span>}
          </div>
          <div className="flex items-center gap-1">
            {!file && <Toggle open={open} onClick={() => setOpen((o) => !o)} label={name} />}
            <span className="min-w-0 flex-1 truncate text-[length:var(--fs-xs)] text-faint" title={path}>
              {path}
            </span>
            <RevealButton path={path} />
          </div>
        </div>
        <button type="button" className="grid h-7 w-7 shrink-0 place-items-center rounded-lg text-muted hover:bg-ink-800 hover:text-paper" aria-label="Close What is this?" title="Close" onClick={() => showWhatIs(null)}>
          ✕
        </button>
      </div>
      {area && !saved && <p className="mt-1.5 text-[length:var(--fs-sm)] text-paper-dim">{area.why_not}</p>}
      {open && <ItemTree path={path} rootSize={size} />}
      <div className="mt-2.5">
        {saved ? (
          <p className="flex flex-wrap items-center gap-x-2 text-[length:var(--fs-sm)] font-semibold text-ok" role="status">
            Saved. {saved.text}
            {saved.a === "disposable" && !file && (
              <button type="button" className={actionBtn} title={cleanUpTitle(path)} onClick={() => void cleanUp(path)}>
                Review it now
              </button>
            )}
          </p>
        ) : (
          <WhatIsButtons path={path} onAnswered={(text, a) => setSaved({ text, a })} />
        )}
      </div>
    </section>
  );
}
