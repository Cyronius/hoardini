import { useEffect, useRef, useState, type CSSProperties } from "react";
import { useStore } from "../store";
import { hopFor, type Act, type MascotState } from "./mascot/acts";
import { useMascotStage } from "./mascot/cue";
import { GIRTH_MIN, girthFor, usedFraction, type Volume } from "./mascot/girth";
import { usePoke } from "./mascot/usePoke";
import "./mascot/mascot.css";

export type { MascotState };

const INK = "#4a3a52";
const SKIN = "#f6cfae";
const SKIN_SHADE = "#eab98f";
const HAIR = "#2f2430";
const CHEEK = "#ff9f95";
const SIDE_L = { "--side": 1 } as CSSProperties;
const SIDE_R = { "--side": -1 } as CSSProperties;

const ACTION: Record<MascotState, string> = {
  idle: "relaxing",
  shuffle: "doing his pre-bout ritual",
  think: "stomping while he thinks",
  tada: "celebrating",
  look: "looking around with his magnifying glass",
  carry: "carrying a box",
  worry: "worried that the drive is nearly full",
  nap: "napping because everything is clean",
};

const star = (x: number, y: number, r: number) =>
  `M${x} ${y - r}L${x + r * 0.28} ${y - r * 0.28}L${x + r} ${y}L${x + r * 0.28} ${y + r * 0.28}L${x} ${y + r}L${x - r * 0.28} ${y + r * 0.28}L${x - r} ${y}L${x - r * 0.28} ${y - r * 0.28}Z`;

function useWhoosh(rx: number, known: boolean): boolean {
  const prev = useRef<number | null>(null);
  const [n, setN] = useState(0);
  useEffect(() => {
    const was = prev.current;
    prev.current = known ? rx : null;
    if (was != null && known && rx < was - 0.2) setN((k) => k + 1);
  }, [rx, known]);
  useEffect(() => {
    if (!n) return;
    const t = setTimeout(() => setN(0), 1600);
    return () => clearTimeout(t);
  }, [n]);
  return n > 0;
}

export function SumoSvg({ act, volume, className = "", label }: { act: Act; volume?: Volume; className?: string; label?: string }) {
  const used = usedFraction(volume);
  const rx = girthFor(used ?? 0.5);
  const whoosh = useWhoosh(rx, used != null);
  const scale = rx / GIRTH_MIN;
  const legOff = 5 + rx * 0.35;
  const armOff = rx - 2;
  const belt = 6.6 * scale;
  const stroke = { stroke: INK, strokeWidth: 1.1, strokeLinejoin: "round" as const };
  const style = { "--jh": `${hopFor(rx).height}px` } as CSSProperties;

  const leg = (side: "l" | "r") => (
    <g className="s-pos" style={{ transform: `translateX(${side === "l" ? -legOff : legOff}px)` }}>
      <g className={`s-leg s-leg-${side}`} style={side === "l" ? SIDE_L : SIDE_R}>
        <rect x="27" y="42" width="10" height="16" rx="5" fill={SKIN} {...stroke} />
        <ellipse cx="32" cy="58.4" rx="6.6" ry="3" fill={SKIN_SHADE} {...stroke} />
      </g>
      <g className={`s-dust s-dust-${side}`} style={side === "l" ? SIDE_L : SIDE_R} fill="var(--color-paper-dim)">
        <circle cx={side === "l" ? 24 : 40} cy="59.5" r="2.4" />
        <circle cx={side === "l" ? 20.5 : 43.5} cy="58" r="1.7" />
        <circle cx={side === "l" ? 27 : 37} cy="56.5" r="1.4" />
      </g>
    </g>
  );

  const arm = (side: "l" | "r") => (
    <g className="s-pos" style={{ transform: `translateX(${side === "l" ? -armOff : armOff}px)` }}>
      <g className={`s-arm s-arm-${side}`} style={side === "l" ? SIDE_L : SIDE_R}>
        <ellipse cx="32" cy="39" rx="4.2" ry="8" fill={SKIN} {...stroke} />
        {side === "r" && (
          <g className="s-lens">
            <path d="M32 46.2v3" stroke={INK} strokeWidth="1.8" strokeLinecap="round" />
            <circle cx="32" cy="52.6" r="3.6" fill="#9fdcff" fillOpacity="0.55" {...stroke} />
            <path d="M30.4 51.4a2 2 0 0 1 1.6 -1.2" stroke="#fffdf8" strokeWidth="0.8" fill="none" strokeLinecap="round" />
          </g>
        )}
      </g>
    </g>
  );

  return (
    <svg
      viewBox="0 0 64 64"
      overflow="visible"
      className={`sumo sumo-${act}${whoosh ? " sumo-whoosh" : ""} ${className}`}
      style={style}
      {...(label ? { role: "img", "aria-label": label } : { "aria-hidden": true })}
    >
      {label && <title>{label}</title>}
      <ellipse className="s-shadow" cx="32" cy="61.2" rx={rx + 4} ry="1.7" fill={INK} opacity="0.13" />
      <g className="s-body">
        {leg("l")}
        {leg("r")}
        <g className="s-torso">
          <g className="s-pos" style={{ transform: `scaleX(${scale})`, transformOrigin: "32px 38px", transformBox: "view-box" }}>
            <ellipse cx="32" cy="37" rx={GIRTH_MIN} ry="15" fill={SKIN} {...stroke} vectorEffect="non-scaling-stroke" />
            <path d="M23.4 42.6 Q32 46.2 40.6 42.6 L38.4 48.4 Q32 52.2 25.6 48.4 Z" fill="var(--color-brass)" stroke={INK} strokeWidth="1" strokeLinejoin="round" vectorEffect="non-scaling-stroke" />
            <path d="M25.2 39.6 Q32 43.6 38.8 39.6" stroke={SKIN_SHADE} strokeWidth="1.3" strokeLinecap="round" fill="none" vectorEffect="non-scaling-stroke" />
            <circle cx="32" cy="35.6" r="0.9" fill={INK} opacity="0.55" />
          </g>
          <path d="M29.5 48.6 L30.1 53.4 H33.9 L34.5 48.6" fill="var(--color-brass)" stroke={INK} strokeWidth="1" strokeLinejoin="round" />
          <g className="s-sagari" fill="none" strokeWidth="1" strokeLinecap="round" style={{ stroke: "color-mix(in srgb, var(--color-brass) 62%, #000)" }}>
            {[-3, -1.5, 0, 1.5, 3].map((d) => (
              <path key={d} d={`M${32 + d} 49.4 q${d * 0.12} 2.6 ${d * 0.2} 5.2`} />
            ))}
          </g>
          <g className="s-box">
            <path d="M24.6 36.4h14.8l-1 9.8H25.6z" fill="#e8c48f" {...stroke} />
            <path d="M24.6 36.4l1.6 -2.4h11.6l1.6 2.4" fill="#d6a965" {...stroke} />
            <path d="M32 36.4v9.8" stroke={INK} strokeWidth="0.6" opacity="0.4" />
          </g>
          {arm("l")}
          {arm("r")}
          <g className="s-creak" fill="none" stroke={INK} strokeWidth="1" strokeLinecap="round">
            {[-1, 1].map((d) => {
              const x = 32 + d * (belt + 0.8);
              return <path key={d} d={`M${x} 47.4l${d * 2.4} -1.3M${x + d * 0.3} 49.4h${d * 2.8}M${x} 51.4l${d * 2.4} 1.3`} />;
            })}
          </g>
          <g className="s-head">
            <circle cx="32" cy="20" r="9" fill={SKIN} {...stroke} />
            <g fill={HAIR} stroke="var(--color-line)" strokeWidth="0.7" strokeLinejoin="round">
              <path d="M23.2 17.6 A9 9 0 0 1 40.8 17.6 Q32 14.2 23.2 17.6 Z" />
              <ellipse cx="32" cy="9.6" rx="3.4" ry="2.5" />
            </g>
            <g className="s-eyes" fill={INK}>
              <ellipse cx="28" cy="20.4" rx="1.3" ry="1.7" />
              <ellipse cx="36" cy="20.4" rx="1.3" ry="1.7" />
              <circle cx="28.45" cy="19.75" r="0.42" fill="#fffdf8" />
              <circle cx="36.45" cy="19.75" r="0.42" fill="#fffdf8" />
            </g>
            <g className="s-brows-calm" fill="none" stroke={INK} strokeWidth="1.3" strokeLinecap="round">
              <path d="M26.2 17.2 q1.8 -1.1 3.6 0" />
              <path d="M34.2 17.2 q1.8 -1.1 3.6 0" />
            </g>
            <g fill="none" stroke={INK} strokeWidth="1.3" strokeLinecap="round">
              <g className="s-eyes-happy">
                <path d="M26.4 21 q1.6 -2.6 3.2 0" />
                <path d="M34.4 21 q1.6 -2.6 3.2 0" />
              </g>
              <g className="s-eyes-shut">
                <path d="M26.5 20.2 q1.5 1.5 3 0" />
                <path d="M34.5 20.2 q1.5 1.5 3 0" />
              </g>
              <g className="s-eyes-x" strokeWidth="1.1">
                <path d="M26.9 19.3 l2.2 2.2 M29.1 19.3 l-2.2 2.2" />
                <path d="M34.9 19.3 l2.2 2.2 M37.1 19.3 l-2.2 2.2" />
              </g>
              <g className="s-brows" strokeWidth="1.1">
                <path d="M25.8 16.9 l3.6 1.2" />
                <path d="M38.2 16.9 l-3.6 1.2" />
              </g>
              <g className="s-brows-up" strokeWidth="1">
                <path d="M26 16.6 q2 -1.4 4 0" />
                <path d="M34 16.6 q2 -1.4 4 0" />
              </g>
            </g>
            <circle cx="24.6" cy="23.6" r="2.4" fill={CHEEK} opacity="0.72" />
            <circle cx="39.4" cy="23.6" r="2.4" fill={CHEEK} opacity="0.72" />
            <path className="s-smile" d="M29.8 25.2 q2.2 2 4.4 0" stroke={INK} strokeWidth="1.2" strokeLinecap="round" fill="none" />
            <path className="s-grin" d="M29.4 24.6 q2.6 4 5.2 0 z" fill={INK} stroke={INK} strokeWidth="0.8" strokeLinejoin="round" />
            <path className="s-frown" d="M29.9 26.9 q2.1 -1.9 4.2 0" stroke={INK} strokeWidth="1.2" strokeLinecap="round" fill="none" />
            <g className="s-grit">
              <rect x="29" y="24.4" width="6" height="2.6" rx="0.8" fill="#fffdf8" stroke={INK} strokeWidth="0.9" />
              <path d="M29.2 25.7 h5.6 M31 24.5 v2.4 M33 24.5 v2.4" stroke={INK} strokeWidth="0.5" />
            </g>
            <ellipse className="s-mouth-o" cx="32" cy="26" rx="1.5" ry="1.9" fill={INK} />
            <path className="s-sweat" d="M41.6 12.6 q2.1 3 0 4.3 q-2.1 -1.3 0 -4.3 z" fill="#9fdcff" stroke={INK} strokeWidth="0.6" />
            <g className="s-stars-orbit" style={{ transform: "translate(32px, 8.6px) scale(1, 0.38)", transformOrigin: "0 0", transformBox: "view-box" }}>
              <g className="s-stars" fill="#f6b73c" stroke={INK} strokeWidth="0.5">
                <path d={star(13, 0, 2.6)} />
                <path d={star(0, 13, 2)} />
                <path d={star(-13, 0, 2.6)} />
                <path d={star(0, -13, 2)} />
              </g>
            </g>
          </g>
        </g>
      </g>
      <g className="s-zzz" fill={INK} fontFamily="var(--font-sans)" fontWeight="800">
        <text className="s-z s-z-1" x="43" y="14" fontSize="5.5">z</text>
        <text className="s-z s-z-2" x="47.5" y="9.5" fontSize="4.4">z</text>
        <text className="s-z s-z-3" x="51" y="5.5" fontSize="3.4">z</text>
      </g>
      <g className="s-salt" fill="#fffdf8" stroke={INK} strokeWidth="0.5">
        <circle className="s-grain s-grain-1" cx="50" cy="20" r="1.2" />
        <circle className="s-grain s-grain-2" cx="50" cy="20" r="1" />
        <circle className="s-grain s-grain-3" cx="50" cy="20" r="1.3" />
        <circle className="s-grain s-grain-4" cx="50" cy="20" r="0.9" />
      </g>
      <g className="s-sparks" fill="#f6b73c">
        <path className="s-spark s-spark-1" d="M8 14l1.3 3 3 1.3-3 1.3L8 22.6l-1.3-3-3-1.3 3-1.3z" />
        <path className="s-spark s-spark-2" d="M56 10l1.1 2.6 2.6 1.1-2.6 1.1L56 17.4l-1.1-2.6-2.6-1.1 2.6-1.1z" />
      </g>
      <g className="s-whoosh" fill="none" stroke={INK} strokeWidth="0.9" strokeLinecap="round" opacity="0.7">
        {(["l", "r"] as const).map((side) => {
          const x = side === "l" ? 32 - rx - 2 : 32 + rx + 2;
          const d = side === "l" ? -1 : 1;
          return (
            <g key={side} className="s-puffs" style={side === "l" ? SIDE_L : SIDE_R}>
              <path className="s-puff s-puff-1" d={`M${x} 33 q${d * 2.5} -1.4 ${d * 5} 0`} />
              <path className="s-puff s-puff-2" d={`M${x} 38 q${d * 3} -1.4 ${d * 6} 0`} />
              <path className="s-puff s-puff-3" d={`M${x} 43 q${d * 2.5} -1.4 ${d * 5} 0`} />
            </g>
          );
        })}
      </g>
    </svg>
  );
}

function describe(state: MascotState, volume: Volume | undefined): string {
  const used = usedFraction(volume);
  return `Hoardini, a sumo wrestler${volume && used != null ? ` — ${volume.root.replace(/\\$/, "")} is ${Math.round(used * 100)}% full` : ""}, ${ACTION[state]}`;
}

export function Mascot({ state, volume, className = "" }: { state: MascotState; volume?: Volume; className?: string }) {
  const away = useStore((s) => s.mascotAway);
  const cue = useMascotStage((s) => s.cue);
  const headerWork = useMascotStage((s) => s.headerWork);
  const hold = useMascotStage((s) => s.hold);
  const quietTadaUntil = useMascotStage((s) => s.quietTadaUntil);
  const home = useRef<HTMLButtonElement>(null);

  const girth = hold ?? volume;
  const rx = girthFor(usedFraction(girth) ?? 0.5);
  let base: Act = state === "tada" && Date.now() < quietTadaUntil ? "idle" : state;
  if (headerWork && (base === "idle" || base === "shuffle")) base = "think";
  const shown = cue?.act ?? base;
  const { playing, poke } = usePoke(rx, shown === "idle" && !away, home);
  const label = describe(state, volume);

  return (
    <button
      ref={home}
      type="button"
      data-mascot-home=""
      aria-label={`${label}. Poke me!`}
      title={`${label}. Poke me!`}
      onClick={poke}
      className={`sumo-button shrink-0 rounded-xl ${className}`}
      style={away ? { visibility: "hidden" } : undefined}
    >
      <SumoSvg act={playing ?? shown} volume={girth} className="h-full w-full" />
    </button>
  );
}
