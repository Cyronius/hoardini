import { Streamdown, defaultRehypePlugins } from "streamdown";
import { code } from "@streamdown/code";

// Drop the "raw" plugin so HTML in LLM or web output renders as literal text (UI-MD-NO-HTML); keep sanitize + harden for URLs.
const { raw: _raw, ...safeRehype } = defaultRehypePlugins;
const REHYPE = Object.values(safeRehype);
const PLUGINS = { code };

export function Markdown({ text, streaming }: { text: string; streaming?: boolean }) {
  return (
    <Streamdown skipHtml rehypePlugins={REHYPE} plugins={PLUGINS} isAnimating={!!streaming}>
      {text}
    </Streamdown>
  );
}
