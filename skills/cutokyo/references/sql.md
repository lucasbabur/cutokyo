# Cutokyo SQL

`cutokyo_sql_query` takes `sql` (one `SELECT` or `WITH … SELECT`) and optional
`max_rows` (default 100, max 500). It cuts text cells at 4,000 characters and
stops a query after 10 seconds, so aggregate in SQL rather than pulling rows.

Only these tables are readable: `sessions`, `projects`, `turns`, `messages`,
`tool_calls`, `agent_runs`, `usage_values`, `summaries`, `summary_sessions`,
`price_snapshots`, `quota_windows`, `context_breakdowns`. List a table's columns with
`SELECT name FROM pragma_table_info('<table>')`.

Values that cost digging:

- `sessions.harness`: `claude_code`, `codex`, `opencode`.
- `tool_calls.state`: `succeeded`, `failed`, `running`, `cancelled`, `unknown`. The
  error text of a failure is in `output_json`.
- `usage_values` holds one row per `metric` and `value`. The metrics are
  `input_tokens`, `output_tokens`, `cache_read_tokens`, `cache_write_tokens` and
  `provider_cost_micros` (millionths of a USD). A metric with no rows is unknown:
  report it as "unknown", never as zero.
- Every `*_epoch` column is unix seconds.

A session's project directory:
`SELECT p.path FROM sessions s JOIN projects p USING (project_id) WHERE s.session_id = '<id>'`.

Tool names are native. Claude's `Bash` is Codex's `shell`. MCP tools are
`mcp__<server>__<tool>` in Claude and `mcp:<server>:<tool>` or `<server>_<tool>`
elsewhere. Find the forms in use with
`SELECT DISTINCT tool_name FROM tool_calls WHERE tool_name LIKE '%<tool>%'`.

Replace every `<placeholder>` before running. Each example is limited to this month.
Change `'start of month'` for another window.

```sql
-- Failure rate per tool
SELECT s.harness, t.tool_name, count(*) AS calls, sum(t.state = 'failed') AS failed,
       round(100.0 * sum(t.state = 'failed') / count(*), 1) AS fail_pct
FROM tool_calls t JOIN sessions s USING (session_id)
WHERE t.started_at_epoch >= CAST(strftime('%s', 'now', 'start of month') AS INTEGER)
GROUP BY 1, 2 ORDER BY failed DESC LIMIT 30;

-- Failures per MCP server (two of three forms: mcp__server__tool and mcp:server:tool;
-- find server_tool names with the DISTINCT query above and add them by hand)
SELECT CASE WHEN tool_name LIKE 'mcp\_\_%' ESCAPE '\'
            THEN substr(tool_name, 6, instr(substr(tool_name, 6), '__') - 1)
            ELSE substr(tool_name, 5, instr(substr(tool_name, 5), ':') - 1) END AS server,
       count(*) AS calls, sum(state = 'failed') AS failed
FROM tool_calls
WHERE (tool_name LIKE 'mcp\_\_%' ESCAPE '\' OR tool_name LIKE 'mcp:%')
  AND started_at_epoch >= CAST(strftime('%s', 'now', 'start of month') AS INTEGER)
GROUP BY 1 ORDER BY failed DESC;

-- Error patterns of one tool (every name form), with an example input
SELECT substr(output_json, 1, 200) AS error, count(*) AS n,
       min(substr(input_json, 1, 200)) AS example_input
FROM tool_calls
WHERE tool_name LIKE '%<tool>' AND state = 'failed'
  AND started_at_epoch >= CAST(strftime('%s', 'now', 'start of month') AS INTEGER)
GROUP BY 1 ORDER BY n DESC LIMIT 10;

-- Tokens and cost per harness
SELECT s.harness, count(DISTINCT s.session_id) AS sessions,
       sum(CASE WHEN u.metric = 'input_tokens' THEN u.value END) AS input,
       sum(CASE WHEN u.metric = 'output_tokens' THEN u.value END) AS output,
       sum(CASE WHEN u.metric = 'provider_cost_micros' THEN u.value END) / 1e6 AS usd
FROM sessions s LEFT JOIN usage_values u ON u.session_id = s.session_id
WHERE s.started_at_epoch >= CAST(strftime('%s', 'now', 'start of month') AS INTEGER)
GROUP BY 1;
```

An audit answer is done when it names the time window, the harnesses covered and the
name forms merged, and quotes one error per failure pattern. A usage answer is done
when it names the time window and the harnesses covered.
