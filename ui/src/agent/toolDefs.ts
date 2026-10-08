import type { StandardTool } from "@itkennel/lm-ag-ui";

export const PROPOSE_TOOLS = ["propose_delete", "propose_archive", "propose_compress", "propose_dedupe"] as const;
export const isProposeTool = (name: string | undefined): boolean => !!name && (PROPOSE_TOOLS as readonly string[]).includes(name);

const strArray = { type: "array", items: { type: "string" } };

export const proposeToolDefs: Record<(typeof PROPOSE_TOOLS)[number], StandardTool> = {
  propose_delete: {
    name: "propose_delete",
    description: "Propose permanently deleting paths. Nothing runs until the user approves the card.",
    parameters: { type: "object", properties: { paths: strArray }, required: ["paths"] },
  },
  propose_archive: {
    name: "propose_archive",
    description: "Propose copying paths to a destination, verifying, then deleting the source.",
    parameters: {
      type: "object",
      properties: { paths: strArray, destination: { type: "string", description: "Destination name" } },
      required: ["paths", "destination"],
    },
  },
  propose_compress: {
    name: "propose_compress",
    description: "Propose compressing a path (zip, 7z, tar.zst, or in-place WOF / LZNT1).",
    parameters: {
      type: "object",
      properties: { path: { type: "string" }, format: { type: "string" } },
      required: ["path", "format"],
    },
  },
  propose_dedupe: {
    name: "propose_dedupe",
    description: "Propose removing duplicate copies of a file, keeping one.",
    parameters: {
      type: "object",
      properties: { group: { type: "string" }, keep: { type: "string" }, mode: { type: "string" } },
      required: ["group", "keep", "mode"],
    },
  },
};
