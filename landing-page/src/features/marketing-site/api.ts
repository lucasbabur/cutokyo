import type {
  AnalyzeCutokyoContextCommandInput,
  AnalyzeCutokyoContextCommandResult,
} from "@/infrastructure/fastapi/commands";
import { analyzeCutokyoContextCommand } from "@/infrastructure/fastapi/commands";

function buildCutokyoAnalysisRequest(contextPayload: string): AnalyzeCutokyoContextCommandInput {
  return {
    actor: {
      displayName: "Public analyzer visitor",
      id: "public-analyzer",
      team: "public-site",
    },
    contextWindowTokens: 200000,
    messages: [
      {
        content: contextPayload,
        role: "user",
      },
    ],
    resources: [],
    target: {
      endpoint: "/cutokyo/context/analyze",
      model: "context-analysis-only",
      provider: "cutokyo",
    },
  };
}

export async function analyzeCutokyoContext(
  contextPayload: string,
  signal?: AbortSignal,
): Promise<AnalyzeCutokyoContextCommandResult> {
  return analyzeCutokyoContextCommand(buildCutokyoAnalysisRequest(contextPayload), signal);
}

export type CutokyoAnalysisResult = AnalyzeCutokyoContextCommandResult;
export type CutokyoAnalysisStatus = "analyzing" | "error" | "idle" | "live";
