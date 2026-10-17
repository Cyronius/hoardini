export function TipLine() {
  return (
    <p
      className="flex items-center gap-1.5 text-[length:var(--fs-xs)] text-muted"
      title="KUAF, the maintainer's local NPR member station. Not affiliated with Hoardini."
    >
      <span className="text-brass" aria-hidden="true">
        ♥
      </span>
      <span>
        Like this?{" "}
        <a className="font-semibold text-brass underline underline-offset-2 hover:no-underline" href="https://www.kuaf.com/donate" target="_blank" rel="noopener noreferrer">
          Tip
        </a>{" "}
        my local NPR station
      </span>
    </p>
  );
}
