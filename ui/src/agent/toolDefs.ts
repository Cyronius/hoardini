import type { StandardTool } from "@itkennel/lm-ag-ui";

export const PROPOSE_TOOLS = ["propose_delete", "propose_archive", "propose_compress", "propose_dedupe", "propose_system_cleanup", "propose_online_only"] as const;
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
    description: "Propose compressing paths (zip, 7z, tar.zst, or in-place WOF / LZNT1).",
    parameters: {
      type: "object",
      properties: { paths: strArray, format: { type: "string" } },
      required: ["paths", "format"],
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
  propose_system_cleanup: {
    name: "propose_system_cleanup",
    description: "Propose cleanups only Windows can do; one administrator prompt when approved.",
    parameters: { type: "object", properties: { operations: strArray }, required: [] },
  },
  propose_online_only: {
    name: "propose_online_only",
    description: "Propose making synced files online-only: the local copy is freed, the file stays in the cloud.",
    parameters: { type: "object", properties: { paths: strArray }, required: ["paths"] },
  },
};
