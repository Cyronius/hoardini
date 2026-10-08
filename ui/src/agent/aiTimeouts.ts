export const DEFAULT_AI_TIMEOUT_SECONDS = 120;
export const DEFAULT_AGENT_MAX_STEPS = 20;
// lm-ag-ui's watchdog hard-aborts; the app's keep-waiting prompt handles slow models, so this is only a leak guard.
export const WATCHDOG_BACKSTOP_MS = 6 * 60 * 60 * 1000;

export function resolveStallMs(timeoutSeconds: number | null | undefined): number {
  const s =
    typeof timeoutSeconds === "number" && Number.isFinite(timeoutSeconds) && timeoutSeconds > 0
      ? Math.floor(timeoutSeconds)
      : DEFAULT_AI_TIMEOUT_SECONDS;
  return s * 1000;
}

export function resolveMaxSteps(v: number | null | undefined): number {
  return typeof v === "number" && Number.isFinite(v) && v > 0 ? Math.floor(v) : DEFAULT_AGENT_MAX_STEPS;
}
