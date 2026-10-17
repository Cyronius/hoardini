import { Component, type ReactNode } from "react";
import { Home } from "../home/Home";
import { useReview } from "../review/state";

// Without it one throw in Home blanks the whole window, the header and settings included.
class HomeBoundary extends Component<{ onRetry?: () => Promise<void>; children: ReactNode }, { error: Error | null }> {
  state: { error: Error | null } = { error: null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error) {
    console.error("Home failed", error);
  }

  render() {
    const { error } = this.state;
    if (!error) return this.props.children;
    const retry = () => void Promise.resolve(this.props.onRetry?.()).finally(() => this.setState({ error: null }));
    return (
      <div className="min-h-0 flex-1 overflow-y-auto px-5 pt-7">
        <div role="alert" className="mx-auto max-w-2xl grid gap-1 text-[length:var(--fs-sm)]">
          <p className="text-[length:var(--fs-md)] font-bold text-paper">Something went wrong on this screen.</p>
          <p className="break-words text-paper-dim">{error.message || String(error)}</p>
          <button type="button" className="mt-2 justify-self-start font-bold text-brass hover:underline" onClick={retry}>
            Try again
          </button>
        </div>
      </div>
    );
  }
}

const reopen = () => useReview.getState().open();

export function Shell() {
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <HomeBoundary onRetry={reopen}>
        <Home />
      </HomeBoundary>
    </div>
  );
}
