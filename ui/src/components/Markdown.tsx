import { Streamdown, defaultRehypePlugins } from "streamdown";
import { code } from "@streamdown/code";

// Drop the "raw" plugin so HTML in LLM or web output renders as literal text (UI-MD-NO-HTML); keep sanitize + harden for URLs.
const { raw: _raw, ...safeRehype } = defaultRehypePlugins;
const REHYPE = Object.values(safeRehype);
const PLUGINS = { code };

export function Markdown({ text, streaming }: { text: string; streaming?: boolean }) {
  return (
    <div className="md">
      {/* Incomplete-markdown repair is only for the live stream: on finished text it cut everything after a bare "<" ("a<b and c"). */}
      <Streamdown skipHtml rehypePlugins={REHYPE} plugins={PLUGINS} isAnimating={!!streaming} parseIncompleteMarkdown={!!streaming}>
        {text}
      </Streamdown>
    </div>
  );
}
