import { useCallback, useEffect, useRef, useState, type RefObject } from "react";
import { ACT_MS, FIDGETS, isFat, nextCombo, pokeAct, reducedMotion, type Combo, type OneShot } from "./acts";

const QUIET_MS = 3 * 60_000;
const JOLT_AT: Partial<Record<OneShot, number>> = { "hop-heavy": 520, shiko: 720, dizzy: 1250 };

function jolt(el: Element | null | undefined, delay: number) {
  el?.animate([{ transform: "none" }, { transform: "translateY(1.5px)" }, { transform: "translateY(-0.5px)" }, { transform: "none" }], {
    duration: 240,
    delay,
    easing: "ease-out",
  });
}

export function usePoke(rx: number, canFidget: boolean, home: RefObject<HTMLElement | null>) {
  const [playing, setPlaying] = useState<OneShot | null>(null);
  const current = useRef<OneShot | null>(null);
  const combo = useRef<Combo>({ count: 0, at: -Infinity });
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);

  const play = useCallback((act: OneShot) => {
    clearTimeout(timer.current);
    current.current = act;
    setPlaying(act);
    timer.current = setTimeout(() => {
      current.current = null;
      if (act === "dizzy" || act === "sulk") combo.current = { count: 0, at: -Infinity };
      setPlaying(null);
    }, ACT_MS[act]);
  }, []);
  useEffect(() => () => clearTimeout(timer.current), []);

  const poke = () => {
    combo.current = nextCombo(combo.current, performance.now());
    const act = pokeAct(combo.current.count, rx, current.current, reducedMotion());
    if (!act) return;
    play(act);
    const at = JOLT_AT[act];
    if (at != null && isFat(rx)) jolt(home.current?.closest("header"), at);
  };

  const idle = canFidget && !playing;
  useEffect(() => {
    if (!idle) return;
    let t: ReturnType<typeof setTimeout> | undefined;
    let quietSince = Date.now();
    const mark = () => {
      quietSince = Date.now();
    };
    // One timer, and only while the window is focused and visible.
    const arm = () => {
      clearTimeout(t);
      t = undefined;
      if (document.hidden || !document.hasFocus()) return;
      t = setTimeout(fire, quietSince + QUIET_MS + Math.random() * 60_000 - Date.now());
    };
    const fire = () => {
      if (Date.now() - quietSince < QUIET_MS) return arm();
      play(FIDGETS[Math.floor(Math.random() * FIDGETS.length)]!);
    };
    const onFocus = () => {
      mark();
      arm();
    };
    const onBlur = () => {
      clearTimeout(t);
      t = undefined;
    };
    arm();
    window.addEventListener("focus", onFocus);
    window.addEventListener("blur", onBlur);
    document.addEventListener("visibilitychange", arm);
    window.addEventListener("pointerdown", mark, { capture: true, passive: true });
    window.addEventListener("keydown", mark, { capture: true, passive: true });
    return () => {
      clearTimeout(t);
      window.removeEventListener("focus", onFocus);
      window.removeEventListener("blur", onBlur);
      document.removeEventListener("visibilitychange", arm);
      window.removeEventListener("pointerdown", mark, { capture: true });
      window.removeEventListener("keydown", mark, { capture: true });
    };
  }, [idle, play]);

  return { playing, poke };
}
