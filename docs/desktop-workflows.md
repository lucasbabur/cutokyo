# Desktop workflows

Onboarding starts with no harness selected. **Browse without installing** saves
that choice without changing native capture configuration. To enable capture,
preview a harness, review its exact targets, acknowledge plaintext local storage,
and confirm installation. Selecting a checkbox alone installs nothing. Finish
rechecks each selected integration before saving it. Configuration verified does
not mean live capture verified; restart the harness and wait for native evidence.
Recover follows recorded installation or cleanup intent. Removal deletes only
unchanged Cutokyo-owned entries and retains modified user entries with an issue.

Automatic capture setup works on macOS, Linux and Windows, for any installed
version of Claude Code, Codex and OpenCode. Parsers were tested on Claude Code
`2.1.278`, Codex `0.153.4` and OpenCode `1.18.28`; those versions and newer ones
are captured fully, and older ones are captured with unknown-version coverage.
Missing binaries, unsafe paths or uninspectable configuration make automatic
setup unavailable; Browse still works. Cutokyo finds harnesses on `PATH` and in
their usual install directories, so an app started from Finder or the Start
menu sees them too. On Windows, Claude Code hooks use exec form and Codex hooks
are quoted for `cmd.exe`, so paths with spaces work. Current Codex releases only
run hooks the user has trusted; setup trusts Cutokyo's own entries through the
Codex App Server, the same way Codex's `/hooks` review does.

**Tool calls** lists every tool your agents called, with harnesses, call count,
success rate, failures and last use. Filter by name, harness or date and sort by
calls, failures or success rate. Open a tool to read its calls newest first,
starting with failures when there are any, each with its native input, output,
duration and session.

**Agent tools** is the place to manage installed MCP servers, skills, hooks,
plugins, and instruction files. Search by name or source, filter by harness or
kind, and choose **Manage** to read the actual source. Editable installations
support saving changes and removal with a retained recovery copy. Cutokyo-owned
capture and search entries explain why they cannot be edited here.

A skill or supported MCP server can be installed into another harness as an
independent copy. The preview names the destination and explains unavailable
formats or credential substitutions. Existing installations are never
overwritten. Shared sources show every affected harness; removing one shared
source affects all of them. Hook and plugin formats are harness-specific and
are not advertised as portable.

**Sessions** supports literal terms and exact phrases, relevance or newest
ordering, match excerpts, filters, and paginated results. Returning from session
detail keeps the search and page. **Today** means the machine's local calendar
day. Resume preserves the exact native identity and recorded project directory,
then opens a visible interactive terminal: Kitty, WezTerm, Ghostty, Alacritty,
Foot or st when installed, otherwise Terminal.app on macOS or a new console
window on Windows. Missing terminal, harness, or display
access fails with a retryable explanation. Success means the native process
acknowledged a real TTY and survived startup. It does not prove session activation;
check the terminal for native login or session errors. Codex hook session IDs
alone are not verified App Server thread IDs and cannot authorize resume.

**Data controls** separates saving a retention policy from deleting history.
Create a backup with one action at the default private location, then review
and confirm a verified restore without copying a digest. Read the scope and
recovery details in [Local backups and restore](docs/backup-and-restore.md).

**Health** offers scoped retries and a previewed local diagnostic report.
Unavailable updater or proxy services disclose the reason rather
than implying a working provider or listener. The guardrails feature is not
included; local redaction, explicit egress consent, and file/database integrity
checks remain.
