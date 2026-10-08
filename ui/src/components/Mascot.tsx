import { GIRTH_MIN, girthFor, usedFraction, type Volume } from "./mascot/girth";
import "./mascot/mascot.css";

export type MascotState = "idle" | "shuffle" | "think" | "tada";

const INK = "#4a3a52";
const SKIN = "#f6cfae";
const SKIN_SHADE = "#eab98f";
const HAIR = "#2f2430";
const CHEEK = "#ff9f95";

const ACTION: Record<MascotState, string> = { idle: "relaxing", shuffle: "doing his pre-bout ritual", think: "stomping while he thinks", tada: "celebrating" };

export function Mascot({ state, volume, className = "" }: { state: MascotState; volume?: Volume; className?: string }) {
  const used = usedFraction(volume);
  const rx = girthFor(used ?? 0.5);
  const scale = rx / GIRTH_MIN;
  const legOff = 5 + rx * 0.35;
  const armOff = rx - 2;
  const label = `Hoardini, a sumo wrestler${volume && used != null ? ` — ${volume.root.replace(/\\$/, "")} is ${Math.round(used * 100)}% full` : ""}, ${ACTION[state]}`;
  const stroke = { stroke: INK, strokeWidth: 1.1, strokeLinejoin: "round" as const };

  const leg = (side: "l" | "r") => (
    <g className="s-pos" style={{ transform: `translateX(${side === "l" ? -legOff : legOff}px)` }}>
      <g className={`s-leg s-leg-${side}`}>
        <rect x="27" y="42" width="10" height="16" rx="5" fill={SKIN} {...stroke} />
        <ellipse cx="32" cy="58.4" rx="6.6" ry="3" fill={SKIN_SHADE} {...stroke} />
      </g>
      <g className={`s-dust s-dust-${side}`} fill="var(--color-paper-dim)">
        <circle cx={side === "l" ? 24 : 40} cy="59.5" r="2.4" />
        <circle cx={side === "l" ? 20.5 : 43.5} cy="58" r="1.7" />
        <circle cx={side === "l" ? 27 : 37} cy="56.5" r="1.4" />
      </g>
    </g>
  );

  const arm = (side: "l" | "r") => (
    <g className="s-pos" style={{ transform: `translateX(${side === "l" ? -armOff : armOff}px)` }}>
      <g className={`s-arm s-arm-${side}`}>
        <ellipse cx="32" cy="39" rx="4.2" ry="8" fill={SKIN} {...stroke} />
      </g>
    </g>
  );

  return (
    <svg viewBox="0 0 64 64" className={`sumo sumo-${state} ${className}`} role="img" aria-label={label}>
      <title>{label}</title>
      {leg("l")}
      {leg("r")}
      <g className="s-torso">
        <g className="s-pos" style={{ transform: `scaleX(${scale})`, transformOrigin: "32px 38px", transformBox: "view-box" }}>
          <ellipse cx="32" cy="37" rx={GIRTH_MIN} ry="15" fill={SKIN} {...stroke} vectorEffect="non-scaling-stroke" />
          <path d="M23.4 42.6 Q32 46.2 40.6 42.6 L38.4 48.4 Q32 52.2 25.6 48.4 Z" fill="var(--color-brass)" stroke={INK} strokeWidth="1" strokeLinejoin="round" vectorEffect="non-scaling-stroke" />
          <circle cx="32" cy="38.4" r="0.9" fill={INK} opacity="0.55" />
        </g>
        {arm("l")}
        {arm("r")}
        <g className="s-head">
          <circle cx="32" cy="20" r="9" fill={SKIN} {...stroke} />
          <g fill={HAIR} stroke="var(--color-line)" strokeWidth="0.7" strokeLinejoin="round">
            <path d="M23.2 17.6 A9 9 0 0 1 40.8 17.6 Q32 14.2 23.2 17.6 Z" />
            <ellipse cx="32" cy="9.6" rx="3.4" ry="2.5" />
          </g>
          <g className="s-eyes" fill={INK}>
            <ellipse cx="28" cy="20.4" rx="1.3" ry="1.7" />
            <ellipse cx="36" cy="20.4" rx="1.3" ry="1.7" />
          </g>
          <g className="s-eyes-happy" fill="none" stroke={INK} strokeWidth="1.3" strokeLinecap="round">
            <path d="M26.4 21 q1.6 -2.6 3.2 0" />
            <path d="M34.4 21 q1.6 -2.6 3.2 0" />
          </g>
          <circle cx="24.6" cy="23.8" r="2.1" fill={CHEEK} opacity="0.75" />
          <circle cx="39.4" cy="23.8" r="2.1" fill={CHEEK} opacity="0.75" />
          <path className="s-smile" d="M29.8 25.2 q2.2 2 4.4 0" stroke={INK} strokeWidth="1.2" strokeLinecap="round" fill="none" />
          <path className="s-grin" d="M29.4 24.6 q2.6 4 5.2 0 z" fill={INK} stroke={INK} strokeWidth="0.8" strokeLinejoin="round" />
        </g>
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
    </svg>
  );
}
