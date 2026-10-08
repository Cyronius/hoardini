export type MascotState = "idle" | "shuffle" | "think" | "tada";

const INK = "#4a3a52";

export function Mascot({ state, className = "" }: { state: MascotState; className?: string }) {
  return (
    <svg
      viewBox="0 0 64 64"
      className={`mascot mascot-${state} ${className}`}
      role="img"
      aria-label={`Hoardini the magician is ${state === "shuffle" ? "shuffling" : state === "think" ? "thinking" : state === "tada" ? "celebrating" : "idle"}`}
    >
      <g className="m-cards">
        <rect className="m-card-a" x="4" y="10" width="9" height="12" rx="1.6" fill="#fff6ee" stroke={INK} strokeWidth="1" />
        <rect className="m-card-b" x="9" y="9" width="9" height="12" rx="1.6" fill="#ffd9cf" stroke={INK} strokeWidth="1" />
        <rect className="m-card-c" x="14" y="11" width="9" height="12" rx="1.6" fill="#fff6ee" stroke={INK} strokeWidth="1" />
      </g>
      <g className="m-dots" fill={INK}>
        <circle className="m-dot-1" cx="47" cy="12" r="2" />
        <circle className="m-dot-2" cx="54" cy="9" r="2.4" />
        <circle className="m-dot-3" cx="61" cy="6" r="2.8" />
      </g>
      <g className="m-sparks" fill="#f6b73c">
        <path className="m-spark m-spark-1" d="M10 20l1.4 3.2L14.6 24.6 11.4 26 10 29.2 8.6 26 5.4 24.6 8.6 23.2z" />
        <path className="m-spark m-spark-2" d="M54 18l1.2 2.8L58 22l-2.8 1.2L54 26l-1.2-2.8L50 22l2.8-1.2z" />
        <path className="m-spark m-spark-3" d="M32 3l1 2.2L35.2 6.2 33 7.2 32 9.4 31 7.2 28.8 6.2 31 5.2z" />
      </g>
      <g className="m-body">
        <ellipse cx="22" cy="55" rx="5" ry="2.6" fill={INK} />
        <ellipse cx="42" cy="55" rx="5" ry="2.6" fill={INK} />
        <g className="m-arm-r">
          <path d="M44 44 L54 35" stroke={INK} strokeWidth="2.2" strokeLinecap="round" />
          <path d="M54 31.2l1 2 2.2.4-1.6 1.6.4 2.2-2-1-2 1 .4-2.2-1.6-1.6 2.2-.4z" fill="#f6b73c" stroke={INK} strokeWidth="0.6" />
          <circle cx="45" cy="44" r="3.4" fill="#fff6ee" stroke={INK} strokeWidth="1" />
        </g>
        <g className="m-arm-l">
          <circle cx="19" cy="45" r="3.4" fill="#fff6ee" stroke={INK} strokeWidth="1" />
        </g>
        <circle cx="32" cy="42" r="15" fill="#fff6ee" stroke={INK} strokeWidth="1.2" />
        <g className="m-eyes" fill={INK}>
          <ellipse cx="26.5" cy="40" rx="1.9" ry="2.3" />
          <ellipse cx="37.5" cy="40" rx="1.9" ry="2.3" />
        </g>
        <circle cx="22.5" cy="45" r="2.6" fill="#ffb3a7" opacity="0.7" />
        <circle cx="41.5" cy="45" r="2.6" fill="#ffb3a7" opacity="0.7" />
        <path className="m-mouth-smile" d="M29 46 q3 3 6 0" stroke={INK} strokeWidth="1.5" strokeLinecap="round" fill="none" />
        <ellipse className="m-mouth-open" cx="32" cy="47" rx="2.6" ry="2.2" fill={INK} />
        <g className="m-hat">
          <rect x="22" y="9" width="20" height="17" rx="3" fill={INK} />
          <rect x="22" y="19.5" width="20" height="4.2" fill="var(--color-brass)" />
          <ellipse cx="32" cy="27" rx="14" ry="3.4" fill={INK} />
          <path d="M33 13l.9 1.9 2 .3-1.5 1.4.4 2-1.8-1-1.8 1 .4-2-1.5-1.4 2-.3z" fill="#f6b73c" />
        </g>
      </g>
    </svg>
  );
}
