import type {
  SessionFilters,
  SessionMatch,
  SessionRecord,
} from "../contracts.js";

export const DEFAULT_SESSION_FILTERS: SessionFilters = {
  text: "",
  queryMode: "terms",
  sort: "relevance",
  offset: 0,
  limit: 50,
  todayStart: null,
  harness: "all",
  project: "",
  branch: "",
  dateRange: "all",
  tool: "",
  skill: "",
  agent: "",
};

/** Date.setHours uses the machine's calendar, including daylight-saving rules. */
export function localMidnight(now: Date): string {
  const start = new Date(now);
  start.setHours(0, 0, 0, 0);
  return start.toISOString();
}

function queryTerms(text: string): string[] {
  return text.match(/[\p{L}\p{N}\p{M}]+/gu) ?? [];
}

function normalize(text: string): string {
  return text.normalize("NFD").replace(/\p{M}/gu, "").toLocaleLowerCase();
}

/**
 * Fixtures exercise literal-word, metadata and message-boundary workflows.
 * Ranking and non-Latin tokenization are approximations; bundled SQLite tests
 * establish production unicode61 and BM25 behavior.
 */
export function sessionMatches(
  session: SessionRecord,
  filters: SessionFilters,
): { matched: boolean; score: number; matches: SessionMatch[] } {
  const terms = queryTerms(filters.text);
  if (filters.text.trim() === "")
    return { matched: true, score: 0, matches: [] };
  if (terms.length === 0) return { matched: false, score: 0, matches: [] };
  const fields: [SessionMatch["source"], string, number][] = [
    ["title", session.title ?? "", 8],
    ["project", session.project ?? "", 5],
    ["branch", session.branch ?? "", 3],
    [
      "native_id",
      `${session.nativeSessionKey} ${session.nativeResumeId ?? ""} ${session.id}`,
      6,
    ],
    ...session.timeline.map(
      (entry): [SessionMatch["source"], string, number] => [
        "transcript",
        entry.body ?? "",
        1,
      ],
    ),
  ];
  const wanted = terms.map(normalize);
  const complete = (text: string) => {
    const words = queryTerms(text).map(normalize);
    return wanted.every((word) => words.includes(word));
  };
  fields.sort(
    (left, right) => Number(complete(right[1])) - Number(complete(left[1])),
  );
  const tokenFields = fields.map(([, text]) => queryTerms(text).map(normalize));
  const fieldMatch = (tokens: readonly string[]) =>
    filters.queryMode === "phrase"
      ? tokens.some((_, index) =>
          wanted.every((word, offset) => tokens[index + offset] === word),
        )
      : wanted.some((word) => tokens.includes(word));
  const matched =
    filters.queryMode === "phrase"
      ? tokenFields.some(fieldMatch)
      : wanted.every((word) =>
          tokenFields.some((tokens) => tokens.includes(word)),
        );
  const matches: SessionMatch[] = [];
  let score = 0;
  fields.forEach(([source, text, weight], index) => {
    const tokens = tokenFields[index] ?? [];
    if (!fieldMatch(tokens)) return;
    score +=
      (weight *
        wanted.reduce(
          (total, word) =>
            total + tokens.filter((token) => token === word).length,
          0,
        )) /
      Math.max(1, tokens.length);
    if (matches.length >= 3 || matches.some((match) => match.source === source))
      return;
    const lower = normalize(text);
    const first = Math.min(
      ...wanted
        .map((word) => lower.indexOf(word))
        .filter((position) => position >= 0),
    );
    const characters = Array.from(text);
    const at = Array.from(
      text.slice(0, Number.isFinite(first) ? first : 0),
    ).length;
    const start = Math.max(0, at - 70);
    const end = Math.min(characters.length, start + 300);
    matches.push({
      source,
      text: `${start > 0 ? "…" : ""}${characters.slice(start, end).join("")}${end < characters.length ? "…" : ""}`,
      terms,
    });
  });
  return { matched, score, matches: matched ? matches : [] };
}
