import { useState } from "react";
import { useStore } from "../store";
import { btnDanger, btnSecondary, ErrorLine, errMsg, Section } from "./ui";

export function ClearHistorySection() {
  const clearHistory = useStore((s) => s.clearHistory);
  const busy = useStore((s) => s.chatBusy);
  const [confirming, setConfirming] = useState(false);
  const [working, setWorking] = useState(false);
  const [done, setDone] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const clear = async () => {
    setWorking(true);
    try {
      await clearHistory();
      setError(null);
      setDone(true);
      setConfirming(false);
    } catch (e) {
      setError(errMsg(e));
    } finally {
      setWorking(false);
    }
  };

  return (
    <Section title="Chat history">
      <p className="text-[length:var(--fs-sm)] text-faint">
        Deletes the chat text. Memories, proposals, jobs, and the archive list stay.
      </p>
      {confirming ? (
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-[length:var(--fs-sm)] font-semibold">Delete every chat message?</span>
          <button className={btnDanger} disabled={working || busy} onClick={() => void clear()}>
            {working ? "Clearing..." : "Delete messages"}
          </button>
          <button className={btnSecondary} disabled={working} onClick={() => setConfirming(false)}>
            Cancel
          </button>
        </div>
      ) : (
        <div className="flex items-center gap-3">
          <button
            className={btnDanger}
            disabled={busy}
            title={busy ? "Wait for the current answer to finish" : undefined}
            onClick={() => {
              setDone(false);
              setConfirming(true);
            }}
          >
            Clear chat history
          </button>
          {done && <span className="text-[length:var(--fs-sm)] font-semibold text-ok">Cleared</span>}
        </div>
      )}
      <ErrorLine error={error} />
    </Section>
  );
}
