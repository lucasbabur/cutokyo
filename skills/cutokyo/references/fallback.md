# When a Cutokyo tool or the server is missing

An MCP server started on an older Cutokyo build lacks `cutokyo_sql_query`. The
whole server can also be disconnected. Both cases use the same history directly.

1. Read the `cutokyo-search` MCP entry. In `~/.claude.json` it is under the
   top-level `mcpServers` (or `projects.<dir>.mcpServers`); in
   `~/.codex/config.toml` it is `[mcp_servers.cutokyo-search]`; in
   `~/.config/opencode/opencode.json` it is under `mcp`. Its `command` is the
   right `cutokyo` binary. A bare `cutokyo` on `PATH` can be an older build that
   refuses the database. The entry's `--data-dir` holds `cutokyo.db`.
2. For SQL, run `sqlite3 -readonly "<data-dir>/cutokyo.db" "<query>"` with the
   queries in [sql.md](sql.md).
3. For the CLI, use that `command` with the entry's `--data-dir`/`--config-file`
   args, and add `--json`:
   - `sessions search --harness codex --sort newest --limit 5` lists recent
     sessions. Add `--query "<words>"` to search.
   - `sessions show <session_id>` gives metadata and usage under `data`.
   - `inventory list` gives `data.items[]` with `kind` (`skill`, `hook`, `mcp`,
     `plugin`, `instruction`), `name`, `harnesses` and `scope` (`user` or
     `project`).

   `--help` on any subcommand shows the rest.

Done when the answer comes from the `cutokyo.db` or binary named in the MCP entry
and meets the completion rule for sessions or inventory (SKILL.md) or for audit and
usage (sql.md).
