---
name: cutokyo
description: Local agent-session history for Claude Code, Codex and OpenCode, through the cutokyo-search MCP. Find a past session or its resume ID ("the session where we…", "what did I do last week"). Audit tool-call failures and errors by tool or MCP server. Measure token usage and cost. Inventory installed skills, hooks, MCP servers and plugins. Load it before answering any of these from memory or by grepping transcripts.
---

# Cutokyo

In Claude Code the tools are `mcp__cutokyo-search__<tool>`. Load deferred ones with
`ToolSearch`. A tool counts as absent when a `select:` search for its exact name
returns nothing.

Pick the branch:

- **Find or resume a session:** `cutokyo_session_search`, then `cutokyo_resume_target`.
  `cutokyo_session_details`, `cutokyo_usage` and `cutokyo_coverage` add one
  session's metadata, tokens and capture gaps.
- **Audit tool calls, measure usage, any other aggregate:** `cutokyo_sql_query`. Read
  [references/sql.md](references/sql.md) first.
- **Inventory:** `cutokyo_inventory`. An inventory answer is done when it lists every
  distinct name of the asked kind, with its harnesses and scope.
- **The tool you need is absent, or the server is disconnected:** read
  [references/fallback.md](references/fallback.md).

## Finding a session

- `text` takes literal words in the language the session was held in. All words
  must match unless `query_mode` is `"phrase"`. With no text, `sort: "newest"`
  lists recent sessions.
- `from`/`until` are full RFC 3339 timestamps, for example `2026-10-20T00:00:00Z`.
  Widen a vague date ("around the 21st") by a day on each side.
- `tool`, `skill`, `harness`, `project` and `branch` filters are exact. A `tool`
  filter only says the session used that tool; per-call outcomes live in SQL.
- Search returns bounded excerpts. To quote what was said, read `messages`
  (`role` is `user` or `assistant`) for that `session_id` through SQL
  ([references/sql.md](references/sql.md)). In a long session, filter by keyword
  and date, and quote the latest decision with the earlier plan as context. Quote only
  the lines the answer needs, because transcripts can hold secrets.
- When several sessions match, name the runners-up with their `session_id`s.
- `cutokyo_resume_target` returns the native ID. The resume commands are
  `claude --resume <id>`, `codex resume <id>` and `opencode -s <id>`, run from the
  session's project directory (the path query is in [references/sql.md](references/sql.md)).
  Give the command to the user rather than running it.

A session answer is done when it gives the title, the UTC start date and the
`session_id`. For a resume request, it also gives the command and its project directory.
