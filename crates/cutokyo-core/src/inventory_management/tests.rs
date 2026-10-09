use super::*;
fn roots(root: &Path) -> InventoryRoots {
    InventoryRoots {
        home: root.into(),
        claude: root.join(".claude"),
        codex: root.join(".codex"),
        opencode: root.join(".config/opencode"),
        opencode_config: None,
        opencode_global: None,
        projects: vec![root.join("project")],
    }
}
fn write(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(path.parent().ok_or("no parent")?)?;
    fs::write(path, bytes)?;
    Ok(())
}
fn item(
    r: &InventoryRoots,
    kind: &str,
    name: &str,
    h: Harness,
) -> InventoryResult<LiveInventoryItem> {
    discover(r)?
        .items
        .into_iter()
        .find(|e| e.kind == kind && e.name == name && e.harnesses.contains(&h))
        .ok_or_else(|| "missing test inventory item".into())
}
#[test]
fn shared_skill_name_blocks_install_from_independent_copy() -> Result<(), Box<dyn std::error::Error>>
{
    let temp = tempfile::tempdir()?;
    let r = roots(temp.path());
    let source = r.claude.join("skills/research");
    write(
        &source.join("SKILL.md"),
        b"---\nname: research\ndescription: Shared research skill\n---\n# Shared source",
    )?;
    let shared = item(&r, "skill", "research", Harness::ClaudeCode)?;
    assert!(shared.harnesses.contains(&Harness::OpenCode));
    let doc = document(&r, &shared.id)?;
    install(
        &r,
        &temp.path().join("recovery"),
        &shared.id,
        &doc.revision,
        Harness::Codex,
    )?;
    let copied = item(&r, "skill", "research", Harness::Codex)?;
    let doc = document(&r, &copied.id)?;
    assert!(doc.install_targets.iter().all(|target| !target.available));
    assert!(
        install(
            &r,
            &temp.path().join("recovery"),
            &copied.id,
            &doc.revision,
            Harness::OpenCode,
        )
        .is_err()
    );
    assert!(!r.opencode.join("skills/research").exists());
    assert_eq!(
        fs::read(source.join("SKILL.md"))?,
        fs::read(r.codex.join("skills/research/SKILL.md"))?,
    );
    Ok(())
}
#[test]
fn skill_edit_copy_assets_remove_refresh_and_stale() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let r = roots(temp.path());
    let bundle = r.claude.join("skills/category/research");
    write(
        &bundle.join("SKILL.md"),
        b"---\nname: research\ndescription: Research skill\n---\n# Original",
    )?;
    write(&bundle.join("scripts/run.py"), b"print('asset')")?;
    write(
        &bundle.join("references/nested/guide.md"),
        b"bundle reference",
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            bundle.join("scripts/run.py"),
            fs::Permissions::from_mode(0o750),
        )?;
    }
    let entry = item(&r, "skill", "research", Harness::ClaudeCode)?;
    let doc = document(&r, &entry.id)?;
    assert!(doc.editable);
    assert!(
        doc.install_targets
            .iter()
            .any(|t| t.harness == Harness::Codex && t.available)
    );
    save(
        &r,
        &temp.path().join("recovery"),
        &entry.id,
        &doc.revision,
        "---\nname: research\ndescription: Edited research\n---\n# Edited",
    )?;
    assert_eq!(
        fs::read_to_string(bundle.join("SKILL.md"))?,
        "---\nname: research\ndescription: Edited research\n---\n# Edited"
    );
    assert!(remove(&r, &temp.path().join("recovery"), &entry.id, &doc.revision).is_err());
    let doc = document(&r, &entry.id)?;
    install(
        &r,
        &temp.path().join("recovery"),
        &entry.id,
        &doc.revision,
        Harness::Codex,
    )?;
    let copied = r.codex.join("skills/research");
    assert_eq!(
        fs::read(copied.join("references/nested/guide.md"))?,
        b"bundle reference"
    );
    assert_eq!(fs::read(copied.join("scripts/run.py"))?, b"print('asset')");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(copied.join("scripts/run.py"))?
                .permissions()
                .mode()
                & 0o777,
            0o750
        );
    }
    assert!(
        install(
            &r,
            &temp.path().join("recovery"),
            &entry.id,
            &doc.revision,
            Harness::Codex
        )
        .is_err()
    );
    assert!(item(&r, "skill", "research", Harness::Codex).is_ok());
    write(&bundle.join("references/nested/guide.md"), b"asset changed")?;
    assert!(
        save(
            &r,
            &temp.path().join("recovery"),
            &entry.id,
            &doc.revision,
            "stale"
        )
        .is_err()
    );
    let doc = document(&r, &entry.id)?;
    remove(&r, &temp.path().join("recovery"), &entry.id, &doc.revision)?;
    assert!(!bundle.exists());
    assert!(item(&r, "skill", "research", Harness::ClaudeCode).is_err());
    assert!(fs::read_dir(temp.path().join("recovery"))?.count() >= 3);
    Ok(())
}
#[test]
fn mcp_conversion_edit_remove_preserves_native_comments_credentials_and_permissions()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let r = roots(temp.path());
    write(&r.home.join(".claude.json"),br#"{"theme":"dark","mcpServers":{"notes":{"command":"node","args":["server.js"],"env":{"API_KEY":"secret"}},"other":{"url":"https://example.com/mcp"}}}"#)?;
    write(
        &r.codex.join("config.toml"),
        b"# keep comment\nmodel = \"gpt-test\"\n[mcp_servers.untouched]\ncommand = \"old\"\n",
    )?;
    write(
        &r.opencode.join("opencode.jsonc"),
        b"{\n // keep this comment\n \"theme\": \"dark\",\n}\n",
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            r.codex.join("config.toml"),
            fs::Permissions::from_mode(0o640),
        )?;
    }
    let entry = item(&r, "mcp", "notes", Harness::ClaudeCode)?;
    let doc = document(&r, &entry.id)?;
    install(
        &r,
        &temp.path().join("recovery"),
        &entry.id,
        &doc.revision,
        Harness::Codex,
    )?;
    let codex = fs::read_to_string(r.codex.join("config.toml"))?;
    assert!(codex.contains("# keep comment"));
    assert!(codex.contains("gpt-test"));
    assert!(codex.contains("secret"));
    assert!(codex.contains("untouched"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(r.codex.join("config.toml"))?
                .permissions()
                .mode()
                & 0o777,
            0o640
        );
    }
    install(
        &r,
        &temp.path().join("recovery"),
        &entry.id,
        &doc.revision,
        Harness::OpenCode,
    )?;
    let open = fs::read_to_string(r.opencode.join("opencode.jsonc"))?;
    assert!(open.contains("// keep this comment"));
    let v = json_value(open.as_bytes())?;
    assert_eq!(v["mcp"]["notes"]["command"], json!(["node", "server.js"]));
    assert_eq!(v["mcp"]["notes"]["environment"]["API_KEY"], "secret");
    let entry = item(&r, "mcp", "notes", Harness::Codex)?;
    let doc = document(&r, &entry.id)?;
    assert!(
        save(
            &r,
            &temp.path().join("recovery"),
            &entry.id,
            &doc.revision,
            "{broken}"
        )
        .is_err()
    );
    save(
        &r,
        &temp.path().join("recovery"),
        &entry.id,
        &doc.revision,
        r#"{"command":"python","args":["server.py"],"env":{"API_KEY":"still-secret"}}"#,
    )?;
    let doc = document(&r, &entry.id)?;
    remove(&r, &temp.path().join("recovery"), &entry.id, &doc.revision)?;
    let v = config_value(
        &r.codex.join("config.toml"),
        &fs::read(r.codex.join("config.toml"))?,
    )?;
    assert!(v["mcp_servers"].get("notes").is_none());
    assert_eq!(v["mcp_servers"]["untouched"]["command"], "old");
    Ok(())
}
#[test]
fn hooks_plugins_instructions_are_real_nodes_and_owned_entries_are_protected()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let r = roots(temp.path());
    let path = r.claude.join("settings.json");
    write(&path,br#"{"unrelated":{"permissions":["keep"]},"hooks":{"PostToolUse":[{"matcher":"Read","hooks":[{"type":"command","command":"user-hook"}]},{"hooks":[{"type":"command","command":"cutokyo hook --cutokyo-owner=cutokyo-claude-v1:00000000-0000-0000-0000-000000000001 --event=PostToolUse"}]}]},"enabledPlugins":{"user@market":true,"cutokyo-capture@local":true}}"#)?;
    write(&r.claude.join("CLAUDE.md"), b"# Instructions")?;
    write(
        &r.opencode.join("plugins/user.ts"),
        b"export default () => ({})",
    )?;
    let entry = item(&r, "hook", "PostToolUse · user-hook", Harness::ClaudeCode)?;
    let doc = document(&r, &entry.id)?;
    assert!(doc.install_targets.iter().all(|t| !t.available));
    save(
        &r,
        &temp.path().join("recovery"),
        &entry.id,
        &doc.revision,
        r#"{"matcher":"Write","hooks":[{"type":"command","command":"new-hook"}]}"#,
    )?;
    let doc = document(&r, &entry.id)?;
    remove(&r, &temp.path().join("recovery"), &entry.id, &doc.revision)?;
    let v = json_value(&fs::read(&path)?)?;
    assert_eq!(
        v["hooks"]["PostToolUse"].as_array().ok_or("array")?.len(),
        1
    );
    assert_eq!(v["unrelated"]["permissions"], json!(["keep"]));
    let owned = item(&r, "hook", "PostToolUse · cutokyo", Harness::ClaudeCode)?;
    let doc = document(&r, &owned.id)?;
    assert!(!doc.editable);
    assert!(remove(&r, &temp.path().join("recovery"), &owned.id, &doc.revision).is_err());
    let user = item(&r, "plugin", "user@market", Harness::ClaudeCode)?;
    let doc = document(&r, &user.id)?;
    remove(&r, &temp.path().join("recovery"), &user.id, &doc.revision)?;
    assert_eq!(
        json_value(&fs::read(&path)?)?["enabledPlugins"]["cutokyo-capture@local"],
        true
    );
    let instruction = item(&r, "instruction", "CLAUDE.md", Harness::ClaudeCode)?;
    let doc = document(&r, &instruction.id)?;
    save(
        &r,
        &temp.path().join("recovery"),
        &instruction.id,
        &doc.revision,
        "# Real instruction edit",
    )?;
    assert_eq!(
        fs::read_to_string(r.claude.join("CLAUDE.md"))?,
        "# Real instruction edit"
    );
    let plugin = item(&r, "plugin", "user.ts", Harness::OpenCode)?;
    let doc = document(&r, &plugin.id)?;
    remove(&r, &temp.path().join("recovery"), &plugin.id, &doc.revision)?;
    assert!(!r.opencode.join("plugins/user.ts").exists());
    Ok(())
}
#[test]
fn duplicate_json_selectors_are_rejected_without_changing_native_source()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let r = roots(temp.path());
    let path = r.home.join(".claude.json");
    let duplicate = br#"{"mcpServers":{"same":{"command":"first"},"same":{"command":"second"}}}"#;
    write(&path, duplicate)?;
    let inventory = discover(&r)?;
    assert!(inventory.items.is_empty());
    assert!(
        inventory
            .notices
            .iter()
            .any(|n| n.contains("Duplicate JSON"))
    );
    assert_eq!(fs::read(&path)?, duplicate);
    write(&path, br#"{"mcpServers":{"safe":{"command":"node"}}}"#)?;
    let entry = item(&r, "mcp", "safe", Harness::ClaudeCode)?;
    let doc = document(&r, &entry.id)?;
    assert!(
        save(
            &r,
            &temp.path().join("recovery"),
            &entry.id,
            &doc.revision,
            r#"{"command":"node","env":{"KEY":"first","KEY":"second"}}"#
        )
        .is_err()
    );
    assert_eq!(
        json_value(&fs::read(&path)?)?["mcpServers"]["safe"]["command"],
        "node"
    );
    Ok(())
}
fn convert(
    from: Harness,
    to: Harness,
    v: &Value,
) -> Result<mcp_convert::Converted, Box<dyn std::error::Error>> {
    Ok(mcp_convert::convert(from, to, v)?)
}
fn has(list: &[String], needle: &str) -> bool {
    list.iter().any(|line| line.contains(needle))
}
#[test]
fn claude_code_docs_mcp_fields_code_claude_com_en_mcp_map_to_codex_and_opencode()
-> Result<(), Box<dyn std::error::Error>> {
    let v = json!({"type":"http","url":"https://example.com/mcp",
        "headers":{"Authorization":"Bearer ${API_TOKEN}","X-Team":"${TEAM_ID}","X-Static":"abc"},
        "headersHelper":"/bin/helper","timeout":600_000,
        "alwaysLoad":true,
        "oauth":{"clientId":"cid","callbackPort":8080,"scopes":"a b","authServerMetadataUrl":"https://auth.example.com/x"}});
    let codex = convert(Harness::ClaudeCode, Harness::Codex, &v)?;
    assert_eq!(codex.value["bearer_token_env_var"], "API_TOKEN");
    assert_eq!(codex.value["env_http_headers"]["X-Team"], "TEAM_ID");
    assert_eq!(codex.value["http_headers"]["X-Static"], "abc");
    assert_eq!(codex.value["http_headers_helper"], "/bin/helper");
    assert!(codex.value.get("headers").is_none() && codex.value["enabled"] == true);
    assert_eq!(
        codex.value["oauth"],
        json!({"client_id":"cid","callback_port":8080})
    );
    assert_eq!(codex.value["scopes"], json!(["a", "b"]));
    assert_eq!(codex.value["tool_timeout_sec"], 600);
    assert!(
        has(&codex.dropped, "alwaysLoad") && has(&codex.dropped, "oauth.authServerMetadataUrl")
    );
    let open = convert(Harness::ClaudeCode, Harness::OpenCode, &v)?;
    assert_eq!(open.value["type"], "remote");
    assert_eq!(
        open.value["headers"]["Authorization"],
        "Bearer {env:API_TOKEN}"
    );
    assert_eq!(open.value["headers"]["X-Team"], "{env:TEAM_ID}");
    assert_eq!(open.value["oauth"], json!({"clientId":"cid","scope":"a b"}));
    assert!(has(&open.dropped, "oauth.callbackPort") && has(&open.dropped, "http_headers_helper"));
    assert!(
        has(&open.dropped, "timeout"),
        "Claude tool timeout has no OpenCode field"
    );
    Ok(())
}
#[test]
fn codex_docs_developers_openai_com_codex_config_reference_fields_map_and_report_the_rest()
-> Result<(), Box<dyn std::error::Error>> {
    let remote = json!({"url":"https://example.com/mcp","bearer_token_env_var":"API_TOKEN",
        "env_http_headers":{"X-Key":"KEY_VAR"},"http_headers":{"X-Static":"abc"},
        "startup_timeout_sec":30.0,"tool_timeout_sec":120,"enabled":true,
        "enabled_tools":["a"],"required":true,"scopes":["read","write"],
        "oauth":{"client_id":"cid","callback_port":9000}});
    let claude = convert(Harness::Codex, Harness::ClaudeCode, &remote)?;
    assert_eq!(claude.value["type"], "http");
    assert_eq!(
        claude.value["headers"]["Authorization"],
        "Bearer ${API_TOKEN}"
    );
    assert_eq!(claude.value["headers"]["X-Key"], "${KEY_VAR}");
    assert_eq!(claude.value["headers"]["X-Static"], "abc");
    assert!(has(&claude.dropped, "startup_timeout_sec") && has(&claude.dropped, "MCP_TIMEOUT"));
    assert_eq!(
        claude.value["timeout"], 120_000,
        "tool_timeout_sec -> Claude per-server timeout ms"
    );
    assert!(has(&claude.dropped, "enabled_tools") && has(&claude.dropped, "required"));
    assert_eq!(
        claude.value["oauth"],
        json!({"clientId":"cid","callbackPort":9000,"scopes":"read write"})
    );
    let open = convert(Harness::Codex, Harness::OpenCode, &remote)?;
    assert_eq!(open.value["timeout"], 30000);
    assert_eq!(
        open.value["headers"]["Authorization"],
        "Bearer {env:API_TOKEN}"
    );
    assert_eq!(open.value["headers"]["X-Key"], "{env:KEY_VAR}");
    assert!(has(&open.dropped, "tool_timeout_sec") && !has(&open.dropped, "startup_timeout_sec"));
    let local = json!({"command":"node","args":["s.js"],"env":{"A":"1"},"env_vars":["TOKEN",{"name":"T2","source":"local"},{"name":"R","source":"remote"}],
        "cwd":"/work","startup_timeout_sec":5,"enabled":false});
    let open = convert(Harness::Codex, Harness::OpenCode, &local)?;
    assert_eq!(open.value["command"], json!(["node", "s.js"]));
    assert_eq!(open.value["environment"]["TOKEN"], "{env:TOKEN}");
    assert_eq!(open.value["environment"]["A"], "1");
    assert_eq!(
        (open.value["timeout"].clone(), open.value["enabled"].clone()),
        (json!(5000), json!(false))
    );
    assert_eq!(
        open.value["cwd"], "/work",
        "OpenCode documents a local-server cwd"
    );
    assert_eq!(open.value["environment"]["T2"], "{env:T2}");
    assert!(has(&open.dropped, "source = \"remote\""));
    let claude = convert(
        Harness::Codex,
        Harness::ClaudeCode,
        &json!({"command":"node","env_vars":["T"]}),
    )?;
    assert_eq!(
        (
            claude.value["type"].clone(),
            claude.value["env"]["T"].clone()
        ),
        (json!("stdio"), json!("${T}"))
    );
    assert!(
        convert(Harness::Codex, Harness::ClaudeCode, &local).is_err(),
        "disabled server must not become enabled"
    );
    Ok(())
}
#[test]
fn opencode_docs_opencode_ai_docs_mcp_servers_fields_map_to_codex_and_claude()
-> Result<(), Box<dyn std::error::Error>> {
    let local = json!({"type":"local","command":["npx","-y","pkg"],"environment":{"TOKEN":"{env:TOKEN}","A":"1"},
        "timeout":4500,"enabled":false,"extra":1});
    let codex = convert(Harness::OpenCode, Harness::Codex, &local)?;
    assert_eq!(
        (codex.value["command"].clone(), codex.value["args"].clone()),
        (json!("npx"), json!(["-y", "pkg"]))
    );
    assert_eq!(codex.value["env_vars"], json!(["TOKEN"]));
    assert_eq!(codex.value["env"], json!({"A": "1"}));
    assert_eq!(
        (
            codex.value["startup_timeout_sec"].clone(),
            codex.value["enabled"].clone()
        ),
        (json!(4.5), json!(false))
    );
    assert!(has(&codex.dropped, "extra"));
    let remote = json!({"type":"remote","url":"https://example.com/mcp",
        "headers":{"Authorization":"Bearer {env:API_TOKEN}"},"oauth":{"clientId":"c","clientSecret":"s","scope":"x y"}});
    let codex = convert(Harness::OpenCode, Harness::Codex, &remote)?;
    assert_eq!(codex.value["bearer_token_env_var"], "API_TOKEN");
    assert!(has(&codex.dropped, "oauth.clientSecret"));
    assert!(!codex.dropped.iter().any(|line| line.contains("s\"")));
    let claude = convert(Harness::OpenCode, Harness::ClaudeCode, &remote)?;
    assert_eq!(
        claude.value["headers"]["Authorization"],
        "Bearer ${API_TOKEN}"
    );
    assert_eq!(
        claude.value["oauth"],
        json!({"clientId":"c","scopes":"x y"})
    );
    assert!(has(&claude.dropped, "oauth.clientSecret"));
    let local = json!({"type":"local","command":["node"],"cwd":"srv"});
    assert_eq!(
        convert(Harness::OpenCode, Harness::Codex, &local)?.value["cwd"],
        "srv"
    );
    assert!(has(
        &convert(Harness::OpenCode, Harness::ClaudeCode, &local)?.dropped,
        "cwd"
    ));
    Ok(())
}
#[test]
fn unexpandable_placeholders_are_reported_not_hidden() -> Result<(), Box<dyn std::error::Error>> {
    let v = json!({"type":"local","command":["node","{file:credentials}"]});
    let codex = convert(Harness::OpenCode, Harness::Codex, &v)?;
    assert!(has(&codex.notes, "args[0]") && has(&codex.notes, "does not expand"));
    let sse = json!({"type":"sse","url":"https://example.com/sse"});
    assert!(has(
        &convert(Harness::ClaudeCode, Harness::Codex, &sse)?.notes,
        "SSE"
    ));
    assert_eq!(
        convert(Harness::ClaudeCode, Harness::OpenCode, &sse)?.value["type"],
        "remote"
    );
    let templated = json!({"type":"http","url":"${BASE}/mcp"});
    assert_eq!(
        convert(Harness::ClaudeCode, Harness::OpenCode, &templated)?.value["url"],
        "{env:BASE}/mcp"
    );
    assert!(convert(Harness::ClaudeCode, Harness::Codex, &templated).is_err());
    Ok(())
}
#[test]
fn install_preview_lists_dropped_fields_and_install_reports_them()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let r = roots(temp.path());
    write(
        &r.codex.join("config.toml"),
        b"[mcp_servers.docs]\nurl = \"https://docs.example.com/mcp\"\nbearer_token_env_var = \"DOCS_TOKEN\"\nstartup_timeout_sec = 30\ntool_timeout_sec = 90\n",
    )?;
    let entry = item(&r, "mcp", "docs", Harness::Codex)?;
    assert_eq!(
        entry.description,
        "Remote MCP server at docs.example.com (streamable HTTP)"
    );
    let doc = document(&r, &entry.id)?;
    let claude = doc
        .install_targets
        .iter()
        .find(|t| t.harness == Harness::ClaudeCode)
        .ok_or("claude")?;
    assert!(claude.available && has(&claude.conversions, "per-server tool timeout"));
    assert!(has(&claude.dropped_fields, "startup_timeout_sec"));
    let codex = doc
        .install_targets
        .iter()
        .find(|t| t.harness == Harness::Codex)
        .ok_or("codex")?;
    assert!(!codex.available);
    assert!(
        codex
            .reason
            .as_deref()
            .is_some_and(|r| r.starts_with("Already installed"))
    );
    let receipt = install(
        &r,
        &temp.path().join("recovery"),
        &entry.id,
        &doc.revision,
        Harness::OpenCode,
    )?;
    assert!(
        receipt.message.contains("Not carried over")
            && receipt.message.contains("tool_timeout_sec")
    );
    let config = json_value(&fs::read(r.opencode.join("opencode.json"))?)?;
    assert_eq!(
        config["mcp"]["docs"]["headers"]["Authorization"],
        "Bearer {env:DOCS_TOKEN}"
    );
    assert_eq!(config["mcp"]["docs"]["timeout"], 30000);
    assert!(!receipt.message.contains("DOCS_TOKEN"));
    let again = item(&r, "mcp", "docs", Harness::OpenCode)?;
    let doc = document(&r, &again.id)?;
    assert!(
        !doc.install_targets
            .iter()
            .find(|t| t.harness == Harness::OpenCode)
            .ok_or("o")?
            .available
    );
    Ok(())
}
#[test]
fn row_descriptions_are_factual_and_free_of_boilerplate() -> Result<(), Box<dyn std::error::Error>>
{
    let temp = tempfile::tempdir()?;
    let r = roots(temp.path());
    write(
        &r.claude.join("skills/pretty/SKILL.md"),
        b"---\nname: pretty\ndescription: >\n  Folded text across\n  two lines: with a colon\n---\n# Body heading",
    )?;
    write(
        &r.claude.join("skills/loose/SKILL.md"),
        b"---\nname: loose\ndescription: Use when: colons break YAML\n---\n# Body",
    )?;
    write(
        &r.claude.join("skills/bare/SKILL.md"),
        b"# Bare heading\ntext",
    )?;
    write(
        &r.claude.join("CLAUDE.md"),
        b"<!-- c -->\n# Project rules\nmore",
    )?;
    write(
        &r.claude.join("settings.json"),
        br#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"API_KEY=sekrit bash /home/u/.claude/hooks/guard.sh --token abc"}]}]},
        "enabledPlugins":{"fmt@market":true}}"#,
    )?;
    write(
        &r.claude
            .join("plugins/cache/market/fmt/1.0/.claude-plugin/plugin.json"),
        br#"{"name":"fmt","description":"Formats code on save"}"#,
    )?;
    write(
        &r.home.join(".claude.json"),
        br#"{"mcpServers":{"local":{"command":"/usr/bin/npx","args":["-y","pkg","--key","sekrit"],"env":{"K":"sekrit"}},"web":{"url":"https://user:pw@api.example.com/mcp?token=sekrit"}}}"#,
    )?;
    let inventory = discover(&r)?;
    let find = |kind: &str, name: &str| {
        inventory
            .items
            .iter()
            .find(|i| i.kind == kind && i.name == name)
            .map(|i| i.description.clone())
    };
    assert_eq!(
        find("skill", "pretty").as_deref(),
        Some("Folded text across two lines: with a colon")
    );
    assert_eq!(
        find("skill", "loose").as_deref(),
        Some("Use when: colons break YAML")
    );
    assert_eq!(find("skill", "bare").as_deref(), Some("Bare heading"));
    assert_eq!(
        find("instruction", "CLAUDE.md").as_deref(),
        Some("Project rules")
    );
    assert_eq!(
        find("plugin", "fmt@market").as_deref(),
        Some("Formats code on save")
    );
    assert_eq!(
        find("hook", "PreToolUse · bash guard.sh").as_deref(),
        Some("On Bash: runs bash guard.sh")
    );
    assert_eq!(
        find("mcp", "local").as_deref(),
        Some("Local MCP server (stdio): npx with 4 arguments")
    );
    assert_eq!(
        find("mcp", "web").as_deref(),
        Some("Remote MCP server at api.example.com (streamable HTTP)")
    );
    for item in &inventory.items {
        for banned in [
            "sekrit",
            "Discovered",
            "Configured presence",
            "Shared native",
            "Supporting asset",
        ] {
            assert!(
                !item.description.contains(banned),
                "{}: {}",
                item.name,
                item.description
            );
        }
    }
    let doc = document(&r, &item(&r, "skill", "pretty", Harness::ClaudeCode)?.id)?;
    assert!(has(&doc.notes, "runtime loading"));
    Ok(())
}
#[cfg(unix)]
#[test]
fn symlink_ancestors_bundle_assets_and_nonregular_destinations_are_rejected()
-> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir()?;
    let r = roots(temp.path());
    write(&r.claude.join("skills/safe/SKILL.md"), b"# safe")?;
    let entry = item(&r, "skill", "safe", Harness::ClaudeCode)?;
    let doc = document(&r, &entry.id)?;
    fs::create_dir_all(temp.path().join("elsewhere"))?;
    symlink(temp.path().join("elsewhere"), &r.codex)?;
    assert!(
        install(
            &r,
            &temp.path().join("recovery"),
            &entry.id,
            &doc.revision,
            Harness::Codex
        )
        .is_err()
    );
    symlink(
        temp.path().join("elsewhere"),
        r.claude.join("skills/safe/link"),
    )?;
    let linked = document(&r, &entry.id)?;
    assert!(has(&linked.notes, "Links and special files"));
    fs::remove_file(r.claude.join("skills/safe/link"))?;
    symlink(
        temp.path().join("elsewhere"),
        temp.path().join("recovery-link"),
    )?;
    assert!(
        save(
            &r,
            &temp.path().join("recovery-link/private"),
            &entry.id,
            &doc.revision,
            "# changed"
        )
        .is_err()
    );
    assert_eq!(
        fs::read_to_string(r.claude.join("skills/safe/SKILL.md"))?,
        "# safe"
    );
    fs::remove_file(&r.codex)?;
    fs::create_dir_all(r.codex.join("config.toml"))?;
    write(
        &r.home.join(".claude.json"),
        br#"{"mcpServers":{"x":{"command":"node"}}}"#,
    )?;
    let entry = item(&r, "mcp", "x", Harness::ClaudeCode)?;
    let doc = document(&r, &entry.id)?;
    assert!(
        install(
            &r,
            &temp.path().join("recovery"),
            &entry.id,
            &doc.revision,
            Harness::Codex
        )
        .is_err()
    );
    Ok(())
}
#[test]
fn bundle_assets_are_not_listed_travel_with_the_skill_and_ignored_dirs_stay_out()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let r = roots(temp.path());
    let bundle = r.claude.join("skills/assets");
    write(
        &bundle.join("SKILL.md"),
        b"---\nname: assets\ndescription: Asset skill\n---\n# Asset skill",
    )?;
    write(
        &bundle.join("references/skill-lifecycle.md"),
        b"# Reference",
    )?;
    write(
        &bundle.join("blueprints/grid-card-assemble.md"),
        b"# Blueprint",
    )?;
    write(&bundle.join("node_modules/dep/.bin/tool"), b"x")?;
    write(&bundle.join(".git/HEAD"), b"ref")?;
    fs::create_dir_all(bundle.join("empty"))?;
    let inventory = discover(&r)?;
    let names: Vec<_> = inventory.items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, vec!["assets"], "assets are not top-level rows");
    assert!(inventory.notices.is_empty(), "{:?}", inventory.notices);
    let parent = item(&r, "skill", "assets", Harness::ClaudeCode)?;
    let doc = document(&r, &parent.id)?;
    assert!(has(&doc.notes, "2 supporting files"));
    assert!(has(&doc.notes, "Dependency, VCS and build directories"));
    assert!(doc.removable && has(&doc.notes, "moves the whole directory"));
    let codex = doc
        .install_targets
        .iter()
        .find(|t| t.harness == Harness::Codex)
        .ok_or("codex")?;
    assert!(codex.available && has(&codex.dropped_fields, "dependency, VCS and build"));
    install(
        &r,
        &temp.path().join("recovery"),
        &parent.id,
        &doc.revision,
        Harness::Codex,
    )?;
    let copy = r.codex.join("skills/assets");
    assert_eq!(
        fs::read_to_string(copy.join("references/skill-lifecycle.md"))?,
        "# Reference"
    );
    assert!(copy.join("empty").is_dir());
    assert!(!copy.join("node_modules").exists() && !copy.join(".git").exists());
    let receipt = remove(&r, &temp.path().join("recovery"), &parent.id, &doc.revision)?;
    assert!(receipt.message.contains("recovery storage"));
    assert!(!bundle.exists(), "bundle left the native tree");
    let moved: Vec<_> = fs::read_dir(temp.path().join("recovery"))?
        .flatten()
        .map(|d| d.path().join("removed-bundle"))
        .filter(|p| p.exists())
        .collect();
    assert_eq!(moved.len(), 1);
    assert!(
        moved[0].join("node_modules/dep/.bin/tool").exists() && moved[0].join(".git/HEAD").exists()
    );
    write(
        &temp.path().join("project/AGENTS.md"),
        b"# Shared project instructions",
    )?;
    let shared = discover(&r)?
        .items
        .into_iter()
        .filter(|item| item.origin == temp.path().join("project/AGENTS.md").display().to_string())
        .collect::<Vec<_>>();
    assert_eq!(shared.len(), 1);
    assert_eq!(shared[0].harnesses, vec![Harness::Codex, Harness::OpenCode]);
    assert_eq!(shared[0].description, "Shared project instructions");
    let doc = document(&r, &shared[0].id)?;
    assert!(has(&doc.notes, "Shared native source") && has(&doc.notes, "Codex, OpenCode"));
    assert!(
        doc.install_targets
            .iter()
            .filter(|target| target.harness != Harness::ClaudeCode)
            .all(|target| !target.available)
    );
    Ok(())
}
#[cfg(unix)]
fn linked_fixture(temp: &Path) -> Result<(InventoryRoots, PathBuf), Box<dyn std::error::Error>> {
    use std::os::unix::fs::symlink;
    let r = roots(temp);
    // npx-skills layout: real copy in a shared store, links from each harness.
    let real = r.home.join("store/linked");
    write(
        &real.join("SKILL.md"),
        b"---\nname: linked\ndescription: Installed through a symlink\n---\n# Linked",
    )?;
    write(&real.join("references/guide.md"), b"# guide")?;
    write(&real.join("node_modules/.bin/tool"), b"x")?;
    fs::create_dir_all(r.claude.join("skills"))?;
    fs::create_dir_all(r.opencode.join("skills"))?;
    symlink(&real, r.claude.join("skills/linked"))?;
    // Relative link, as OpenCode installs create them.
    symlink("../../../store/linked", r.opencode.join("skills/linked"))?;
    symlink(temp.join("gone"), r.claude.join("skills/dangling"))?;
    Ok((r, real))
}
#[cfg(unix)]
#[test]
fn symlinked_skills_are_discovered_described_installed_and_only_unlinked_on_removal()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let (r, real) = linked_fixture(temp.path())?;
    let inventory = discover(&r)?;
    let claude = item(&r, "skill", "linked", Harness::ClaudeCode)?;
    assert_eq!(claude.description, "Installed through a symlink");
    assert_eq!(
        claude.link_target.as_deref(),
        Some(fs::canonicalize(&real)?.to_str().ok_or("utf8")?)
    );
    assert_eq!(
        inventory
            .items
            .iter()
            .filter(|i| i.name == "linked")
            .count(),
        2
    );
    assert!(inventory.items.iter().all(|i| i.name != "dangling"));
    assert_eq!(
        inventory
            .notices
            .iter()
            .filter(|n| n.contains("dangling"))
            .count(),
        1,
        "{:?}",
        inventory.notices
    );
    assert!(
        !inventory
            .notices
            .iter()
            .any(|n| n.contains("Unsafe symlink") || n.contains("bound"))
    );
    let doc = document(&r, &claude.id)?;
    let codex = doc
        .install_targets
        .iter()
        .find(|t| t.harness == Harness::Codex)
        .ok_or("codex")?;
    assert!(codex.available, "{:?}", codex.reason);
    install(
        &r,
        &temp.path().join("recovery"),
        &claude.id,
        &doc.revision,
        Harness::Codex,
    )?;
    let copy = r.codex.join("skills/linked");
    assert!(!fs::symlink_metadata(&copy)?.file_type().is_symlink());
    assert_eq!(
        fs::read_to_string(copy.join("references/guide.md"))?,
        "# guide"
    );
    // Removing the link leaves the shared real directory and the other link intact.
    let doc = document(&r, &claude.id)?;
    let receipt = remove(&r, &temp.path().join("recovery"), &claude.id, &doc.revision)?;
    assert!(receipt.message.contains("link only"));
    assert!(fs::symlink_metadata(r.claude.join("skills/linked")).is_err());
    assert!(real.join("SKILL.md").exists() && real.join("references/guide.md").exists());
    assert!(fs::symlink_metadata(r.opencode.join("skills/linked")).is_ok());
    assert!(item(&r, "skill", "linked", Harness::OpenCode).is_ok());
    // A link whose target directory vanished is reported once and never crashes discovery.
    fs::remove_dir_all(&real)?;
    assert!(discover(&r)?.notices.iter().any(|n| n.contains("dangling")));
    Ok(())
}
#[cfg(unix)]
#[test]
fn linked_skill_edit_writes_through_the_resolved_file_with_revision_checks()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let (r, real) = linked_fixture(temp.path())?;
    let claude = item(&r, "skill", "linked", Harness::ClaudeCode)?;
    let doc = document(&r, &claude.id)?;
    assert!(
        doc.editable && doc.removable,
        "{:?}",
        doc.unavailable_reason
    );
    assert!(has(&doc.notes, "Shared with: Claude Code, OpenCode via"));
    let edited = "---\nname: linked\ndescription: Edited through the link\n---\n# Edited";
    assert!(
        save(
            &r,
            &temp.path().join("recovery"),
            &claude.id,
            "stale",
            edited
        )
        .is_err()
    );
    save(
        &r,
        &temp.path().join("recovery"),
        &claude.id,
        &doc.revision,
        edited,
    )?;
    assert_eq!(
        fs::read_to_string(real.join("SKILL.md"))?,
        edited,
        "written through the resolved file"
    );
    assert!(
        fs::symlink_metadata(r.claude.join("skills/linked"))?
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        item(&r, "skill", "linked", Harness::OpenCode)?.description,
        "Edited through the link"
    );
    Ok(())
}
#[cfg(unix)]
#[test]
fn linked_skill_inside_a_dependency_tree_is_not_editable() -> Result<(), Box<dyn std::error::Error>>
{
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir()?;
    let r = roots(temp.path());
    fs::create_dir_all(r.claude.join("skills"))?;
    // A target inside a dependency tree is never edited.
    let vendored = r.home.join("store/node_modules/pkg-skill");
    write(
        &vendored.join("SKILL.md"),
        b"---\nname: pkg-skill\ndescription: d\n---\n",
    )?;
    symlink(&vendored, r.claude.join("skills/pkg-skill"))?;
    let doc = document(&r, &item(&r, "skill", "pkg-skill", Harness::ClaudeCode)?.id)?;
    assert!(!doc.editable && doc.removable);
    assert!(
        doc.unavailable_reason
            .is_some_and(|r| r.contains("dependency or build"))
    );
    Ok(())
}
#[cfg(unix)]
#[test]
fn links_inside_skills_and_loops_are_one_notice_not_a_wall()
-> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir()?;
    let r = roots(temp.path());
    for name in ["a", "b"] {
        let dir = r.claude.join("skills").join(name);
        write(
            &dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: d\n---\n").as_bytes(),
        )?;
        symlink("/etc/hostname", dir.join("asset-link"))?;
    }
    fs::create_dir_all(r.claude.join("skills/category"))?;
    symlink(
        r.claude.join("skills"),
        r.claude.join("skills/category/loop"),
    )?;
    let inventory = discover(&r)?;
    assert_eq!(
        inventory.items.iter().filter(|i| i.kind == "skill").count(),
        2
    );
    assert_eq!(
        inventory
            .notices
            .iter()
            .filter(|n| n.contains("linked or special"))
            .count(),
        1
    );
    assert!(inventory.notices.iter().any(|n| n.contains("loops")));
    assert!(inventory.notices.len() <= 2, "{:?}", inventory.notices);
    Ok(())
}
#[test]
fn project_roots_nested_skills_and_live_new_entries() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let r = roots(temp.path());
    write(
        &temp.path().join("project/.claude/skills/a/SKILL.md"),
        b"# parent",
    )?;
    write(
        &temp
            .path()
            .join("project/.claude/skills/a/nested/b/SKILL.md"),
        b"# nested",
    )?;
    let items = discover(&r)?.items;
    assert!(items.iter().any(|i| i.name == "b" && i.scope == "project"));
    write(
        &r.codex.join("config.toml"),
        b"[mcp_servers.new]\ncommand = \"new-live\"\n",
    )?;
    assert!(item(&r, "mcp", "new", Harness::Codex).is_ok());
    let custom = temp.path().join("custom/native.jsonc");
    write(
        &custom,
        br#"{"mcp":{"servers":{"custom-server":{"type":"local","command":["node"]}}}}"#,
    )?;
    let mut custom_roots = r.clone();
    custom_roots.opencode_config = Some(custom.clone());
    let entry = item(&custom_roots, "mcp", "custom-server", Harness::OpenCode)?;
    let doc = document(&custom_roots, &entry.id)?;
    save(
        &custom_roots,
        &temp.path().join("recovery"),
        &entry.id,
        &doc.revision,
        r#"{"type":"local","command":["python"]}"#,
    )?;
    assert_eq!(
        json_value(&fs::read(&custom)?)?["mcp"]["servers"]["custom-server"]["command"],
        json!(["python"])
    );
    let global = r.home.join("standard-opencode");
    write(
        &global.join("opencode.json"),
        br#"{"mcp":{"global-server":{"type":"local","command":["node"]}}}"#,
    )?;
    custom_roots.opencode_global = Some(global);
    write(
        &custom_roots.opencode.join("plugins/user.ts"),
        b"export default () => ({})",
    )?;
    assert!(item(&custom_roots, "mcp", "global-server", Harness::OpenCode).is_ok());
    assert!(item(&custom_roots, "plugin", "user.ts", Harness::OpenCode).is_ok());
    let doc = document(&custom_roots, &entry.id)?;
    remove(
        &custom_roots,
        &temp.path().join("recovery"),
        &entry.id,
        &doc.revision,
    )?;
    assert!(
        json_value(&fs::read(&custom)?)?["mcp"]["servers"]
            .get("custom-server")
            .is_none()
    );
    assert!(item(&custom_roots, "mcp", "global-server", Harness::OpenCode).is_ok());
    assert!(item(&custom_roots, "plugin", "user.ts", Harness::OpenCode).is_ok());
    Ok(())
}
#[test]
fn opencode_v2_selectors_plugins_flat_skills_and_install_preserve_siblings()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let r = roots(temp.path());
    let recovery = temp.path().join("recovery");
    let path = r.opencode.join("opencode.jsonc");
    write(&path, br#"{ // preserve
      "theme":"keep", "mcp":{"servers":{"one":{"type":"local","command":["node"]},"two":{"type":"local","command":["python"]}}},
      "plugins":["pkg-a",["pkg-b",{"token":"secret"}],{"package":"pkg-c","options":{"key":"secret"}}]
    }"#)?;
    assert!(item(&r, "mcp", "servers", Harness::OpenCode).is_err());
    let one = item(&r, "mcp", "one", Harness::OpenCode)?;
    let d = document(&r, &one.id)?;
    save(
        &r,
        &recovery,
        &one.id,
        &d.revision,
        r#"{"type":"local","command":["node",""],"enabled":false}"#,
    )?;
    assert_eq!(item(&r, "mcp", "one", Harness::OpenCode)?.state, "disabled");
    assert_eq!(
        json_value(&fs::read(&path)?)?["mcp"]["servers"]["one"]["enabled"],
        false
    );
    let d = document(&r, &one.id)?;
    remove(&r, &recovery, &one.id, &d.revision)?;
    let v = json_value(&fs::read(&path)?)?;
    assert!(v["mcp"]["servers"].get("one").is_none());
    assert_eq!(v["mcp"]["servers"]["two"]["command"], json!(["python"]));
    assert_eq!(v["theme"], "keep");
    let plugin = item(&r, "plugin", "pkg-b", Harness::OpenCode)?;
    let d = document(&r, &plugin.id)?;
    save(
        &r,
        &recovery,
        &plugin.id,
        &d.revision,
        r#"["pkg-b",{"token":"changed"}]"#,
    )?;
    let d = document(&r, &plugin.id)?;
    remove(&r, &recovery, &plugin.id, &d.revision)?;
    let v = json_value(&fs::read(&path)?)?;
    assert_eq!(
        v["plugins"],
        json!(["pkg-a",{"package":"pkg-c","options":{"key":"secret"}}])
    );
    write(
        &r.home.join(".claude.json"),
        br#"{"mcpServers":{"new":{"command":"node","env":{"TOKEN":"kept"}}}}"#,
    )?;
    let source = item(&r, "mcp", "new", Harness::ClaudeCode)?;
    let d = document(&r, &source.id)?;
    install(&r, &recovery, &source.id, &d.revision, Harness::OpenCode)?;
    let v = json_value(&fs::read(&path)?)?;
    assert!(v["mcp"].get("new").is_none());
    assert_eq!(v["mcp"]["servers"]["new"]["environment"]["TOKEN"], "kept");
    assert!(fs::read_to_string(&path)?.contains("// preserve"));
    let flat = r.opencode.join("skills/flat.md");
    write(&flat, b"# Flat skill")?;
    let skill = item(&r, "skill", "flat.md", Harness::OpenCode)?;
    let d = document(&r, &skill.id)?;
    assert!(d.install_targets.iter().all(|t| !t.available));
    save(&r, &recovery, &skill.id, &d.revision, "# Real flat edit")?;
    assert_eq!(fs::read_to_string(&flat)?, "# Real flat edit");
    let d = document(&r, &skill.id)?;
    remove(&r, &recovery, &skill.id, &d.revision)?;
    assert!(!flat.exists());
    assert!(item(&r, "skill", "flat.md", Harness::OpenCode).is_err());
    Ok(())
}
#[test]
fn inline_codex_tables_allow_real_edit_remove_and_install() -> Result<(), Box<dyn std::error::Error>>
{
    let temp = tempfile::tempdir()?;
    let r = roots(temp.path());
    let recovery = temp.path().join("recovery");
    let path = r.codex.join("config.toml");
    write(&path,b"# keep\nmodel = 'keep'\nmcp_servers = { example = { command = 'node' }, other = { command = 'keep' } }\n")?;
    let entry = item(&r, "mcp", "example", Harness::Codex)?;
    let d = document(&r, &entry.id)?;
    save(
        &r,
        &recovery,
        &entry.id,
        &d.revision,
        r#"{"command":"python","env":{"KEY":"secret"}}"#,
    )?;
    let v = config_value(&path, &fs::read(&path)?)?;
    assert_eq!(v["mcp_servers"]["example"]["command"], "python");
    assert_eq!(v["model"], "keep");
    let d = document(&r, &entry.id)?;
    remove(&r, &recovery, &entry.id, &d.revision)?;
    let v = config_value(&path, &fs::read(&path)?)?;
    assert!(v["mcp_servers"].get("example").is_none());
    assert_eq!(v["mcp_servers"]["other"]["command"], "keep");
    write(
        &r.home.join(".claude.json"),
        br#"{"mcpServers":{"new":{"command":"node"}}}"#,
    )?;
    let entry = item(&r, "mcp", "new", Harness::ClaudeCode)?;
    let d = document(&r, &entry.id)?;
    install(&r, &recovery, &entry.id, &d.revision, Harness::Codex)?;
    let v = config_value(&path, &fs::read(&path)?)?;
    assert_eq!(v["mcp_servers"]["new"]["command"], "node");
    assert!(fs::read_to_string(&path)?.contains("# keep"));
    Ok(())
}
#[test]
fn malformed_yaml_and_native_interpolation_fail_without_mutation()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let r = roots(temp.path());
    let recovery = temp.path().join("recovery");
    let path = r.codex.join("skills/yaml/SKILL.md");
    write(
        &path,
        b"---\nname: yaml\ndescription: Valid skill\n---\n# valid",
    )?;
    let entry = item(&r, "skill", "yaml", Harness::Codex)?;
    let d = document(&r, &entry.id)?;
    for invalid in [
        "# No frontmatter",
        "---\nname: yaml\n---\n# missing description",
        "---\nname: WRONG_NAME\ndescription: invalid\n---\n# invalid",
        "---\nname: wrong-name\ndescription: mismatch\n---\n# invalid",
    ] {
        assert!(save(&r, &recovery, &entry.id, &d.revision, invalid).is_err());
        assert!(fs::read_to_string(&path)?.contains("# valid"));
    }
    let invalid = "---\nname: [broken\n---\n# invalid";
    assert!(save(&r, &recovery, &entry.id, &d.revision, invalid).is_err());
    assert!(fs::read_to_string(&path)?.contains("# valid"));
    write(&path, invalid.as_bytes())?;
    let d = document(&r, &entry.id)?;
    assert!(install(&r, &recovery, &entry.id, &d.revision, Harness::ClaudeCode).is_err());
    assert!(!r.claude.join("skills/yaml").exists());
    write(&r.home.join(".claude.json"),br#"{"mcpServers":{"auth":{"url":"https://example.com/mcp","headers":{"Authorization":"Bearer ${API_TOKEN}"}}}}"#)?;
    let entry = item(&r, "mcp", "auth", Harness::ClaudeCode)?;
    let d = document(&r, &entry.id)?;
    install(&r, &recovery, &entry.id, &d.revision, Harness::OpenCode)?;
    assert_eq!(
        json_value(&fs::read(r.opencode.join("opencode.json"))?)?["mcp"]["auth"]["headers"]["Authorization"],
        "Bearer {env:API_TOKEN}"
    );
    assert!(
        save(
            &r,
            &recovery,
            &entry.id,
            &d.revision,
            r#"{"type":"http","url":"file:///tmp/${CONFIG_PATH}"}"#
        )
        .is_err()
    );
    let native = r#"{"type":"http","url":"${API_BASE_URL:-https://example.com}/mcp"}"#;
    save(&r, &recovery, &entry.id, &d.revision, native)?;
    assert_eq!(
        json_value(&fs::read(r.home.join(".claude.json"))?)?["mcpServers"]["auth"]["url"],
        "${API_BASE_URL:-https://example.com}/mcp"
    );
    let d = document(&r, &entry.id)?;
    assert!(d.install_targets.iter().all(|target| !target.available));
    Ok(())
}
#[test]
fn ownership_requires_real_marker_not_unrelated_names_paths_urls_or_args()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let r = roots(temp.path());
    write(&r.home.join(".claude.json"),br#"{"mcpServers":{"cutokyo-community-docs":{"command":"node","args":["/projects/cutokyo-community/user-docs-server.js"],"env":{"KEY":"cutokyo-user-token"}},"remote":{"url":"https://cutokyo.example.com/mcp"},"cutokyo-search":{"command":"cutokyo","args":["mcp"]}}}"#)?;
    for name in ["cutokyo-community-docs", "remote"] {
        let entry = item(&r, "mcp", name, Harness::ClaudeCode)?;
        assert!(!entry.managed_by_cutokyo);
        let d = document(&r, &entry.id)?;
        assert!(d.editable && d.removable);
    }
    let entry = item(&r, "mcp", "cutokyo-search", Harness::ClaudeCode)?;
    let d = document(&r, &entry.id)?;
    assert!(!d.editable && !d.removable);
    assert!(
        save(
            &r,
            &temp.path().join("recovery"),
            &entry.id,
            &d.revision,
            "{}"
        )
        .is_err()
    );
    Ok(())
}
#[cfg(unix)]
#[test]
fn bundle_directory_modes_are_revisioned_preserved_and_readonly_removal_leaves_no_tombstone()
-> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir()?;
    let r = roots(temp.path());
    let recovery = temp.path().join("recovery");
    let bundle = r.codex.join("skills/modes");
    write(
        &bundle.join("SKILL.md"),
        b"---\nname: modes\ndescription: Permissions skill\n---\n# modes",
    )?;
    write(&bundle.join("nested/file.md"), b"asset")?;
    fs::create_dir_all(bundle.join("empty"))?;
    fs::set_permissions(&bundle, fs::Permissions::from_mode(0o750))?;
    fs::set_permissions(bundle.join("nested"), fs::Permissions::from_mode(0o710))?;
    let entry = item(&r, "skill", "modes", Harness::Codex)?;
    let d = document(&r, &entry.id)?;
    install(&r, &recovery, &entry.id, &d.revision, Harness::ClaudeCode)?;
    assert_eq!(
        fs::metadata(r.claude.join("skills/modes"))?
            .permissions()
            .mode()
            & 0o777,
        0o750
    );
    assert_eq!(
        fs::metadata(r.claude.join("skills/modes/nested"))?
            .permissions()
            .mode()
            & 0o777,
        0o710
    );
    assert!(r.claude.join("skills/modes/empty").exists());
    fs::set_permissions(&bundle, fs::Permissions::from_mode(0o500))?;
    assert_ne!(document(&r, &entry.id)?.revision, d.revision);
    assert!(remove(&r, &recovery, &entry.id, &d.revision).is_err());
    let d = document(&r, &entry.id)?;
    remove(&r, &recovery, &entry.id, &d.revision)?;
    assert!(!bundle.exists());
    assert!(fs::read_dir(r.codex.join("skills"))?.next().is_none());
    assert!(item(&r, "skill", "modes", Harness::Codex).is_err());
    for directory in fs::read_dir(&recovery)? {
        let directory = directory?;
        if directory.file_type()?.is_dir() {
            assert_eq!(directory.metadata()?.permissions().mode() & 0o777, 0o700);
            assert_eq!(
                fs::metadata(directory.path().join("intent.json"))?
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }
    Ok(())
}
#[test]
fn mcp_servers_turn_off_and_on_without_losing_configuration()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let r = roots(temp.path());
    let recovery = temp.path().join("recovery");
    let toggle = |name: &str, h: Harness, on: bool| -> InventoryResult<String> {
        let entry = item(&r, "mcp", name, h)?;
        let d = document(&r, &entry.id)?;
        set_enabled(&r, &recovery, &entry.id, &d.revision, on)?;
        Ok(item(&r, "mcp", name, h)?.state)
    };

    // Codex and OpenCode carry a native `enabled` field on the server entry.
    write(
        &r.codex.join("config.toml"),
        b"# keep me\n[mcp_servers.docs]\ncommand = \"docs\"\n",
    )?;
    write(
        &r.opencode.join("opencode.json"),
        br#"{"theme":"keep","mcp":{"docs":{"type":"local","command":["docs"]}}}"#,
    )?;
    assert_eq!(toggle("docs", Harness::Codex, false)?, "disabled");
    let codex = fs::read_to_string(r.codex.join("config.toml"))?;
    assert!(
        codex.contains("# keep me") && codex.contains("enabled = false"),
        "{codex}"
    );
    assert_eq!(toggle("docs", Harness::Codex, true)?, "configured");
    assert!(!fs::read_to_string(r.codex.join("config.toml"))?.contains("enabled"));
    assert_eq!(
        item(&r, "mcp", "docs", Harness::OpenCode)?.state,
        "configured"
    );
    assert_eq!(toggle("docs", Harness::OpenCode, false)?, "disabled");
    let open = json_value(&fs::read(r.opencode.join("opencode.json"))?)?;
    assert_eq!(open["mcp"]["docs"]["enabled"], false);
    assert_eq!(open["theme"], "keep");
    assert_eq!(toggle("docs", Harness::OpenCode, true)?, "configured");
    let open = json_value(&fs::read(r.opencode.join("opencode.json"))?)?;
    assert!(open["mcp"]["docs"].get("enabled").is_none());

    // Claude Code: the server entry is untouched; `deniedMcpServers` is the switch.
    let claude = br#"{"mcpServers":{"docs":{"command":"docs"},"refero":{"url":"https://x"}}}"#;
    write(&r.home.join(".claude.json"), claude)?;
    assert_eq!(toggle("docs", Harness::ClaudeCode, false)?, "disabled");
    assert_eq!(
        item(&r, "mcp", "refero", Harness::ClaudeCode)?.state,
        "configured"
    );
    let settings = r.claude.join("settings.json");
    assert_eq!(
        json_value(&fs::read(&settings)?)?["deniedMcpServers"],
        json!([{"serverName": "docs"}])
    );
    assert_eq!(fs::read(r.home.join(".claude.json"))?, claude.to_vec());
    write(&settings, br#"{"theme":"keep","deniedMcpServers":[{"serverName":"docs"},{"serverUrl":"https://y/*"}]}"#)?;
    assert_eq!(toggle("docs", Harness::ClaudeCode, true)?, "configured");
    let after = json_value(&fs::read(&settings)?)?;
    assert_eq!(
        after["deniedMcpServers"],
        json!([{"serverUrl": "https://y/*"}])
    );
    assert_eq!(after["theme"], "keep");

    // Only user MCP servers have a switch here.
    write(
        &r.home.join(".claude.json"),
        br#"{"mcpServers":{"cutokyo-search":{"command":"cutokyo","args":["mcp"]}}}"#,
    )?;
    let owned = item(&r, "mcp", "cutokyo-search", Harness::ClaudeCode)?;
    let d = document(&r, &owned.id)?;
    assert!(set_enabled(&r, &recovery, &owned.id, &d.revision, false).is_err());
    write(&r.claude.join("CLAUDE.md"), b"# Rules")?;
    let rules = item(&r, "instruction", "CLAUDE.md", Harness::ClaudeCode)?;
    let d = document(&r, &rules.id)?;
    assert!(set_enabled(&r, &recovery, &rules.id, &d.revision, false).is_err());
    Ok(())
}
#[test]
fn search_mcp_is_registered_in_every_detected_agent_and_removed_cleanly()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let r = roots(temp.path());
    let recovery = temp.path().join("recovery");
    let command: Vec<String> = ["/opt/cutokyo", "--data-dir", "/d", "mcp", "serve"]
        .map(str::to_owned)
        .into();
    // Detected agents: Claude (dir), Codex (dir with a commented config), OpenCode V2.
    fs::create_dir_all(&r.claude)?;
    write(&r.codex.join("config.toml"), b"# mine\nmodel = \"x\"\n")?;
    write(
        &r.opencode.join("opencode.jsonc"),
        b"{\n  // keep\n  \"mcp\": {\"servers\": {\"other\": {\"type\": \"local\", \"command\": [\"o\"]}}}\n}\n",
    )?;
    assert!(reconcile_search_mcp(&r, &recovery, &command, true).is_empty());
    let claude = json_value(&fs::read(r.home.join(".claude.json"))?)?;
    assert_eq!(claude["mcpServers"]["cutokyo-search"]["args"][2], "mcp");
    let codex = fs::read_to_string(r.codex.join("config.toml"))?;
    assert!(codex.starts_with("# mine") && codex.contains("[mcp_servers.cutokyo-search]"));
    let open = fs::read_to_string(r.opencode.join("opencode.jsonc"))?;
    assert!(open.contains("// keep"));
    let open = json_value(open.as_bytes())?;
    assert_eq!(
        open["mcp"]["servers"]["cutokyo-search"]["command"][0],
        "/opt/cutokyo"
    );
    assert_eq!(open["mcp"]["servers"]["other"]["command"], json!(["o"]));
    for h in [Harness::ClaudeCode, Harness::Codex, Harness::OpenCode] {
        assert!(item(&r, "mcp", "cutokyo-search", h)?.managed_by_cutokyo);
    }

    // Already in the wanted state: nothing is rewritten and no recovery note is added.
    let snapshot = |r: &InventoryRoots| -> std::io::Result<Vec<Vec<u8>>> {
        Ok(vec![
            fs::read(r.home.join(".claude.json"))?,
            fs::read(r.codex.join("config.toml"))?,
            fs::read(r.opencode.join("opencode.jsonc"))?,
        ])
    };
    let notes = fs::read_dir(&recovery)?.count();
    let before = snapshot(&r)?;
    assert!(reconcile_search_mcp(&r, &recovery, &command, true).is_empty());
    assert_eq!(snapshot(&r)?, before);
    assert_eq!(fs::read_dir(&recovery)?.count(), notes);

    // Turning Agent search off removes only Cutokyo's entry.
    assert!(reconcile_search_mcp(&r, &recovery, &command, false).is_empty());
    let codex = fs::read_to_string(r.codex.join("config.toml"))?;
    assert!(codex.starts_with("# mine") && !codex.contains("cutokyo-search"));
    let open = json_value(&fs::read(r.opencode.join("opencode.jsonc"))?)?;
    assert!(open["mcp"]["servers"].get("cutokyo-search").is_none());
    assert_eq!(open["mcp"]["servers"]["other"]["command"], json!(["o"]));
    assert!(
        json_value(&fs::read(r.home.join(".claude.json"))?)?["mcpServers"]
            .get("cutokyo-search")
            .is_none()
    );

    // Agents that are not installed are never touched, and symlinked configs are refused.
    let other = tempfile::tempdir()?;
    let bare = roots(other.path());
    assert!(reconcile_search_mcp(&bare, &recovery, &command, true).is_empty());
    assert!(!bare.home.join(".claude.json").exists() && !bare.codex.exists());
    #[cfg(unix)]
    {
        fs::create_dir_all(&bare.codex)?;
        let real = other.path().join("real.toml");
        fs::write(&real, b"")?;
        std::os::unix::fs::symlink(&real, bare.codex.join("config.toml"))?;
        let notices = reconcile_search_mcp(&bare, &recovery, &command, true);
        assert!(
            notices.iter().any(|n| n.starts_with("Codex:")),
            "{notices:?}"
        );
        assert!(fs::read(&real)?.is_empty());
    }
    Ok(())
}
