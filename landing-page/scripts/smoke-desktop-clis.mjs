import { spawn } from "node:child_process";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import http from "node:http";
import os from "node:os";
import path from "node:path";

const openAiText = "MOCK_CUTOKYO_OK";
const anthropicText = "MOCK_CLAUDE_OK";
const geminiText = "MOCK_GEMINI_OK";
const repeatLine = "INFO indexing workspace";

const tempHome = await mkdtemp(path.join(os.tmpdir(), "cutokyo-desktop-clis-"));
const codexHome = path.join(tempHome, ".codex");
const claudeSettingsPath = path.join(tempHome, ".claude", "settings.json");
const geminiSettingsPath = path.join(tempHome, ".gemini", "settings.json");
const geminiEnvPath = path.join(tempHome, ".env");
const recoveryReportPath = path.join(tempHome, "cutokyo-recovery-report.txt");
const captures = [];
const otlpExports = [];
const mock = await startMockProvider();
const proxyPort = await findFreePort();
const proxyBaseUrl = `http://127.0.0.1:${proxyPort}`;

let proxy = null;

try {
  await ensureCli("codex", ["--version"]);
  await ensureCli("claude", ["--version"]);
  await ensureCli("npx", ["--yes", "@google/gemini-cli@0.49.0", "--version"], {
    cwd: tempHome,
    env: {
      ...process.env,
      HOME: tempHome,
      USERPROFILE: tempHome,
    },
  });

  await run("cargo", ["build", "-p", "cutokyo-desktop", "--bin", "cutokyo"], {
    cwd: "..",
    env: {
      ...process.env,
      CUTOKYO_WORKOS_AUTHKIT_DOMAIN: `http://127.0.0.1:${mock.port}`,
      CUTOKYO_WORKOS_CLIENT_ID: "client_cutokyo_cli_smoke",
      CUTOKYO_WORKOS_REDIRECT_URI: `${proxyBaseUrl}/cutokyo/auth/callback`,
    },
    timeoutMs: 240_000,
  });

  proxy = spawn(binaryPath(), [], {
    cwd: "..",
    env: proxyEnv(tempHome, mock.port, proxyPort),
    stdio: ["ignore", "pipe", "pipe"],
  });

  const proxyErrors = [];
  proxy.stderr.on("data", (chunk) => proxyErrors.push(chunk.toString()));

  const status = await waitForJson(`${proxyBaseUrl}/cutokyo/status`);
  assert(status.status === "ready", "Cutokyo proxy did not report ready.");
  assert(status.features?.tokenSaving?.available === true, "Token-saving capability was absent.");
  assert(status.features.tokenSaving.active === false, "Token saving activated without consent.");
  const detection = await getJson(`${proxyBaseUrl}/cutokyo/detection`);
  for (const harnessId of ["codex-cli", "claude-code", "gemini-cli"]) {
    assert(
      detection.some((harness) => harness.id === harnessId),
      `Detection omitted ${harnessId}.`,
    );
  }
  const pluginStatus = await getJson(`${proxyBaseUrl}/cutokyo/plugins/status`);
  assert(pluginStatus.enabled === false, "Plugin mutation should start disabled.");
  assert(Array.isArray(pluginStatus.plugins), "Plugin runtime status omitted plugin definitions.");
  const initialSettings = await getJson(`${proxyBaseUrl}/cutokyo/settings`);
  assert(initialSettings.compression === false, "Compression should start opt-in and off.");
  assert(
    initialSettings.providerMutation === false,
    "Provider mutation should require explicit consent.",
  );
  const enabledSettings = await putJson(`${proxyBaseUrl}/cutokyo/settings`, {
    ...initialSettings,
    compression: true,
    providerMutation: true,
  });
  assert(enabledSettings.compression === true, "Compression setting did not persist through PUT.");
  assert(enabledSettings.providerMutation === true, "Provider mutation consent did not persist.");
  const enabledStatus = await getJson(`${proxyBaseUrl}/cutokyo/status`);
  assert(enabledStatus.features.tokenSaving.active === true, "Status omitted active token saving.");
  assert(
    (await getJson(`${proxyBaseUrl}/cutokyo/settings`)).compression === true,
    "Compression setting was not readable after persistence.",
  );

  const activation = await postJson(`${proxyBaseUrl}/cutokyo/activation`, {
    applyChanges: true,
    claudeSettingsPath,
    clients: ["codex", "claude", "gemini"],
    codexConfigPath: path.join(codexHome, "config.toml"),
    cutokyoBaseUrl: proxyBaseUrl,
    enabled: true,
    gatewayFallback: true,
    geminiEnvPath,
    geminiSettingsPath,
  });
  assert(
    activation.clients?.every((client) =>
      [
        "changed",
        "already-active",
        "gateway-fallback-configured",
        "gateway-fallback-ready",
      ].includes(client.status),
    ),
    `Activation did not enable all clients: ${JSON.stringify(activation.clients)}`,
  );

  const promptBody = makeRepeatedPrompt();
  const geminiBaseUrl = await readActivatedEnvValue(geminiEnvPath, "GOOGLE_GEMINI_BASE_URL");
  const codex = await run(
    "codex",
    [
      "exec",
      "--skip-git-repo-check",
      "--dangerously-bypass-approvals-and-sandbox",
      "--cd",
      tempHome,
      "--model",
      "gpt-5-codex",
      "--color",
      "never",
      "-",
    ],
    {
      cwd: tempHome,
      env: {
        ...process.env,
        CODEX_HOME: codexHome,
        HOME: tempHome,
        OPENAI_API_KEY: "sk-cutokyo-smoke",
        USERPROFILE: tempHome,
      },
      input: `Reply exactly ${openAiText} after reading this log.\n${promptBody}`,
      timeoutMs: 120_000,
    },
  );

  const claude = await run(
    "claude",
    [
      "--bare",
      "--settings",
      claudeSettingsPath,
      "--model",
      "claude-3-5-haiku-latest",
      "--tools",
      "",
      "--no-session-persistence",
      "--output-format",
      "json",
      "-p",
      `Reply exactly ${anthropicText} after reading this log.\n${promptBody}`,
    ],
    {
      cwd: tempHome,
      env: {
        ...process.env,
        ANTHROPIC_API_KEY: "sk-ant-cutokyo-smoke",
        HOME: tempHome,
        USERPROFILE: tempHome,
      },
      timeoutMs: 120_000,
    },
  );

  const gemini = await run(
    "npx",
    [
      "--yes",
      "@google/gemini-cli@0.49.0",
      "--model",
      "gemini-3.1-flash-lite",
      "--prompt",
      `Reply exactly ${geminiText} after reading this log.\n${promptBody}`,
      "--output-format",
      "json",
      "--skip-trust",
      "--yolo",
    ],
    {
      cwd: tempHome,
      env: {
        ...process.env,
        GEMINI_API_KEY: "sk-gemini-cutokyo-smoke",
        GOOGLE_GEMINI_BASE_URL: geminiBaseUrl,
        HOME: tempHome,
        USERPROFILE: tempHome,
      },
      timeoutMs: 120_000,
    },
  );

  assert(codex.stdout.includes(openAiText), "Codex did not return the mock response.");
  assert(claude.stdout.includes(anthropicText), "Claude did not return the mock response.");
  assert(gemini.stdout.includes(geminiText), "Gemini did not return the mock response.");

  const openAiCapture = lastCapture("openai-responses");
  const anthropicCapture = lastCapture("anthropic");
  const geminiCapture = lastCapture("gemini");
  assert(openAiCapture, "Codex did not reach the mock OpenAI upstream.");
  assert(anthropicCapture, "Claude did not reach the mock Anthropic upstream.");
  assert(geminiCapture, "Gemini did not reach the mock Gemini upstream.");

  const openAiBody = openAiCapture.body;
  const anthropicBody = anthropicCapture.body;
  const geminiBody = geminiCapture.body;
  const openAiRepeatCount = countOccurrences(openAiBody, repeatLine);
  const anthropicRepeatCount = countOccurrences(anthropicBody, repeatLine);
  const geminiRepeatCount = countOccurrences(geminiBody, repeatLine);

  assert(
    openAiRepeatCount >= 70,
    `Codex user prompt was unexpectedly compressed: ${openAiRepeatCount}.`,
  );
  assert(
    anthropicRepeatCount >= 70,
    `Claude user prompt was unexpectedly compressed: ${anthropicRepeatCount}.`,
  );
  assert(
    geminiRepeatCount >= 70,
    `Gemini user prompt was unexpectedly compressed: ${geminiRepeatCount}.`,
  );
  assert(openAiBody.includes("ERROR keep this line"), "Codex body lost the important line.");
  assert(anthropicBody.includes("ERROR keep this line"), "Claude body lost the important line.");
  assert(geminiBody.includes("ERROR keep this line"), "Gemini body lost the important line.");

  await postProviderRequest(`${proxyBaseUrl}/v1/responses`, {
    input: [
      { content: "Preserve this user prompt.", role: "user" },
      { call_id: "call_cutokyo_smoke", output: promptBody, type: "function_call_output" },
    ],
    model: "gpt-5-codex",
  });
  await postProviderRequest(`${proxyBaseUrl}/v1/messages`, {
    max_tokens: 12,
    messages: [
      { content: "Preserve this user prompt.", role: "user" },
      { content: promptBody, role: "tool" },
    ],
    model: "claude-3-5-haiku-latest",
  });
  await postProviderRequest(`${proxyBaseUrl}/v1beta/models/gemini-3.1-flash-lite:generateContent`, {
    contents: [
      { parts: [{ text: "Preserve this user prompt." }], role: "user" },
      { parts: [{ text: promptBody }], role: "tool" },
    ],
  });

  const directOpenAiCapture = lastCapture("openai-responses");
  const directAnthropicCapture = lastCapture("anthropic");
  const directGeminiCapture = lastCapture("gemini");
  for (const [provider, capture] of [
    ["OpenAI", directOpenAiCapture],
    ["Anthropic", directAnthropicCapture],
    ["Gemini", directGeminiCapture],
  ]) {
    assert(capture, `${provider} direct request did not reach the mock provider.`);
    assert(
      countOccurrences(capture.body, repeatLine) < 80,
      `${provider} tool output was not compressed: ${capture.body.slice(0, 600)}`,
    );
    assert(
      capture.body.includes("ERROR keep this line"),
      `${provider} compression lost the critical error sentinel.`,
    );
  }

  const metrics = await getJson(`${proxyBaseUrl}/cutokyo/metrics`);
  assert(
    metrics.liveRequests >= 6,
    `Expected at least 6 live requests, got ${metrics.liveRequests}.`,
  );
  assert(
    metrics.compressedRequests >= 3,
    `Expected at least 3 compressed requests, got ${metrics.compressedRequests}.`,
  );
  assert(
    metrics.totalEstimatedTokensSaved > 0,
    `Expected saved tokens to be tracked, got ${metrics.totalEstimatedTokensSaved}.`,
  );
  const events = await getJson(`${proxyBaseUrl}/cutokyo/events`);
  const eventStoreStatus = await getJson(`${proxyBaseUrl}/cutokyo/events/status`);
  assert(events.length >= 6, `Expected at least 6 durable events, got ${events.length}.`);
  assert(eventStoreStatus.encryption === "XChaCha20-Poly1305", "Event storage is not encrypted.");
  assert(eventStoreStatus.keySource === "environment", "Event key source was not reported.");
  assert(
    !eventStoreStatus.lastWriteError,
    `Event storage failed: ${eventStoreStatus.lastWriteError}`,
  );
  for (const provider of ["openai-responses", "anthropic-messages", "gemini-generate-content"]) {
    const event = events.find((candidate) => candidate.provider === provider);
    assert(event, `Missing durable event for ${provider}.`);
    assert(
      event.usageSource === "provider-reported",
      `${provider} event did not use provider-reported tokens.`,
    );
    assert(event.usage.totalTokens === 4, `${provider} event token total was not captured.`);
    assert(
      event.attributedUsage.measurement === "estimated-chars-div-4",
      `${provider} event did not label Cutokyo token attribution.`,
    );
    assert(
      event.attributedUsage.totalEstimatedTokens > 0,
      `${provider} event did not attribute request context.`,
    );
    assert(event.contentRecorded === true, `${provider} event omitted governed local messages.`);
    assert(event.traceId?.length === 32, `${provider} event trace ID was not generated.`);
  }
  const sessions = await getJson(`${proxyBaseUrl}/cutokyo/sessions`);
  assert(sessions.length > 0, "Attributed events were not grouped into local sessions.");
  assert(
    sessions.some((session) => session.messages?.some((message) => message.role === "user")),
    "Session detail did not expose a governed user message.",
  );
  assert(
    sessions.some((session) => session.messages?.some((message) => message.role === "assistant")),
    "Session detail did not expose a governed assistant message.",
  );
  assert(
    sessions.some((session) => session.context?.toolResults > 0),
    "Session detail did not aggregate tool-result context.",
  );
  const geminiEvent = events.find((candidate) => candidate.provider === "gemini-generate-content");
  assert(geminiEvent?.cost?.status === "priced", "Gemini event cost was not priced.");
  assert(
    geminiEvent.cost.pricingVersion === "2026-07-09",
    "Gemini event did not snapshot its pricing version.",
  );
  assert(
    geminiEvent.cost.totalCostNanosUsd === 4750,
    `Unexpected Gemini event cost: ${geminiEvent.cost.totalCostNanosUsd}.`,
  );
  const rawEventLog = await readFile(
    path.join(tempHome, ".cutokyo", "events", "operations.jsonl"),
    "utf8",
  );
  assert(
    rawEventLog.includes("XChaCha20-Poly1305"),
    "Durable telemetry was not encrypted at rest.",
  );
  assert(!rawEventLog.includes(repeatLine), "Durable telemetry retained raw tool output.");
  assert(!rawEventLog.includes("preview"), "Durable telemetry retained a content preview.");
  assert(!rawEventLog.includes(openAiText), "Encrypted event log leaked assistant content.");
  assert(!rawEventLog.includes("gemini-3.1-flash-lite"), "Encrypted event leaked model metadata.");
  const exportStatus = await waitForExport(proxyBaseUrl);
  assert(exportStatus.pendingEvents === 0, "OTLP export left pending events.");
  assert(
    exportStatus.exportedEvents >= events.length,
    `OTLP export count was incomplete: events=${events.length} status=${JSON.stringify(exportStatus)} batches=${otlpExports.length}.`,
  );
  assert(otlpExports.length > 0, "Mock OTLP collector did not receive a batch.");
  const rawOtlp = JSON.stringify(otlpExports);
  assert(rawOtlp.includes("resourceLogs"), "OTLP payload did not use the logs schema.");
  assert(rawOtlp.includes("gen_ai.usage.input_tokens"), "OTLP payload omitted GenAI usage.");
  assert(
    rawOtlp.includes("cutokyo.usage.attributed.tool_definitions.tokens"),
    "OTLP payload omitted attributed context categories.",
  );
  assert(!rawOtlp.includes(repeatLine), "OTLP payload retained raw tool output.");
  assert(!rawOtlp.includes("preview"), "OTLP payload retained a content preview.");

  const crashExit = new Promise((resolve) => proxy.once("exit", resolve));
  proxy.kill("SIGKILL");
  await crashExit;
  proxy = null;
  await waitForCrashRecovery(recoveryReportPath);

  console.log(
    JSON.stringify(
      {
        ok: true,
        anthropicRepeatCount,
        anthropicUrl: anthropicCapture.url,
        anthropicUserAgent: anthropicCapture.headers["user-agent"],
        captureCount: captures.length,
        codexStdout: codex.stdout.trim().split("\n").at(-1),
        claudeResult: parseClaudeResult(claude.stdout),
        crashRecovery: "harness-configuration-restored",
        geminiRepeatCount,
        geminiResult: parseGeminiResult(gemini.stdout),
        geminiUrl: geminiCapture.url,
        geminiUserAgent: geminiCapture.headers["user-agent"],
        durableEventCount: events.length,
        metricsCompressedRequests: metrics.compressedRequests,
        metricsLiveRequests: metrics.liveRequests,
        metricsTokensSaved: metrics.totalEstimatedTokensSaved,
        sessionCount: sessions.length,
        otlpExportCount: otlpExports.length,
        otlpExportedEvents: exportStatus.exportedEvents,
        openAiRepeatCount,
        openAiUrl: openAiCapture.url,
        openAiUserAgent: openAiCapture.headers["user-agent"],
      },
      null,
      2,
    ),
  );

  if (proxyErrors.length > 0) {
    console.error(proxyErrors.join(""));
  }
} finally {
  if (proxy && !proxy.killed) {
    proxy.kill();
  }
  await mock.close();
  if (process.env.CUTOKYO_KEEP_SMOKE_TEMP === "1") {
    console.error(`Preserved smoke directory: ${tempHome}`);
  } else {
    await removeTempDirectory(tempHome);
  }
}

async function removeTempDirectory(directory) {
  const deadline = Date.now() + 10_000;
  while (true) {
    try {
      await rm(directory, { force: true, recursive: true });
      return;
    } catch (error) {
      if (!["EBUSY", "ENOTEMPTY", "EPERM"].includes(error.code) || Date.now() >= deadline) {
        throw error;
      }
      await new Promise((resolve) => setTimeout(resolve, 250));
    }
  }
}

async function waitForCrashRecovery(reportPath) {
  const deadline = Date.now() + 20_000;
  let state = null;
  while (Date.now() < deadline) {
    const [codex, claude, gemini, envText] = await Promise.all([
      readFile(path.join(codexHome, "config.toml"), "utf8").catch(() => ""),
      readFile(claudeSettingsPath, "utf8").catch(() => ""),
      readFile(geminiSettingsPath, "utf8").catch(() => ""),
      readFile(geminiEnvPath, "utf8").catch(() => ""),
    ]);
    state = {
      claudeManaged: claude.includes("ANTHROPIC_BASE_URL"),
      claudeBaseUrl: parseJson(claude).env?.ANTHROPIC_BASE_URL ?? null,
      codexManaged: codex.includes("Cutokyo Codex provider"),
      expectedBaseUrl: proxyBaseUrl,
      geminiBaseUrl:
        envText
          .split("\n")
          .find((line) => line.startsWith("GOOGLE_GEMINI_BASE_URL="))
          ?.split("=", 2)[1] ?? null,
      geminiEnvManaged: envText.includes("Cutokyo Gemini gateway"),
      geminiSettingsManaged: gemini.includes('"selectedType": "gateway"'),
    };
    if (
      !state.claudeManaged &&
      !state.codexManaged &&
      !state.geminiEnvManaged &&
      !state.geminiSettingsManaged
    ) {
      return;
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(
    `Activation watchdog did not restore harness configuration after a crash: ${JSON.stringify(state)} report=${await readFile(reportPath, "utf8").catch(() => "missing")}`,
  );
}

function binaryPath() {
  const executable = process.platform === "win32" ? "cutokyo.exe" : "cutokyo";
  return path.join("target", "debug", executable);
}

function makeRepeatedPrompt() {
  return [...Array.from({ length: 80 }, () => repeatLine), "ERROR keep this line"].join("\n");
}

function proxyEnv(home, port, proxyPort) {
  const env = Object.fromEntries(
    Object.entries(process.env).filter(([key]) => !key.startsWith("CUTOKYO_WORKOS_")),
  );
  return {
    ...env,
    CUTOKYO_ANTHROPIC_MESSAGES_URL: `http://127.0.0.1:${port}/v1/messages`,
    CUTOKYO_GEMINI_BASE_URL: `http://127.0.0.1:${port}`,
    CUTOKYO_OPENAI_MODELS_URL: `http://127.0.0.1:${port}/v1/models`,
    CUTOKYO_OPENAI_RESPONSES_URL: `http://127.0.0.1:${port}/v1/responses`,
    CUTOKYO_API_ADDR: `127.0.0.1:${proxyPort}`,
    CUTOKYO_TOKEN_SAVING_ENABLED: "true",
    CUTOKYO_PROXY_ONLY: "1",
    CUTOKYO_EVENT_ENCRYPTION_KEY: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
    CUTOKYO_RECOVERY_REPORT_PATH: recoveryReportPath,
    OTEL_EXPORTER_OTLP_LOGS_ENDPOINT: `http://127.0.0.1:${port}/v1/logs`,
    OTEL_EXPORTER_OTLP_LOGS_HEADERS: "x-cutokyo-test=present",
    OTEL_EXPORTER_OTLP_LOGS_PROTOCOL: "http/json",
    HOME: home,
    USERPROFILE: home,
  };
}

function findFreePort() {
  const server = http.createServer();
  return new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      const port = address.port;
      server.close((error) => (error ? reject(error) : resolve(port)));
    });
  });
}

async function ensureCli(command, args, options = {}) {
  await run(command, args, { ...options, timeoutMs: 30_000 });
}

function startMockProvider() {
  const server = http.createServer(async (request, response) => {
    const url = new URL(request.url ?? "/", "http://127.0.0.1");
    if (request.method === "GET" && url.pathname === "/health") {
      writeJson(response, { ok: true });
      return;
    }
    if (request.method === "GET" && url.pathname === "/v1/models") {
      capture(request, url, "openai-models", "");
      writeJson(response, {
        data: [{ created: 0, id: "gpt-5-codex", object: "model", owned_by: "openai" }],
        object: "list",
      });
      return;
    }

    const body = await readBody(request);
    if (request.method === "POST" && url.pathname === "/v1/logs") {
      capture(request, url, "otlp-logs", body);
      assert(
        request.headers["x-cutokyo-test"] === "present",
        "OTLP exporter did not forward configured headers.",
      );
      otlpExports.push(parseJson(body));
      writeJson(response, {});
      return;
    }
    if (request.method === "POST" && url.pathname === "/v1/responses") {
      const payload = parseJson(body);
      capture(request, url, "openai-responses", body);
      sendOpenAiStream(response, payload.model);
      return;
    }
    if (request.method === "POST" && url.pathname === "/v1/messages") {
      const payload = parseJson(body);
      capture(request, url, "anthropic", body);
      sendAnthropicStream(response, payload.model);
      return;
    }
    if (request.method === "POST" && url.pathname.startsWith("/v1beta/models/")) {
      capture(request, url, "gemini", body);
      if (url.pathname.endsWith(":streamGenerateContent")) {
        sendGeminiStream(response, modelNameFromGeminiUrl(url));
      } else {
        writeJson(response, geminiResponse(modelNameFromGeminiUrl(url)));
      }
      return;
    }

    writeJson(response, { error: `Unhandled ${request.method} ${request.url}` }, 404);
  });

  return new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      resolve({
        port: address.port,
        close: () =>
          new Promise((closeResolve, closeReject) => {
            server.close((error) => (error ? closeReject(error) : closeResolve()));
          }),
      });
    });
  });
}

function capture(request, url, provider, body) {
  captures.push({
    body,
    headers: request.headers,
    method: request.method,
    provider,
    url: `${url.pathname}${url.search}`,
  });
}

function sendOpenAiStream(response, model) {
  const responseId = "resp_cutokyo_mock";
  const itemId = "msg_cutokyo_mock";
  const createdAt = Math.floor(Date.now() / 1000);
  const output = {
    content: [{ annotations: [], text: openAiText, type: "output_text" }],
    id: itemId,
    role: "assistant",
    status: "completed",
    type: "message",
  };
  const completed = {
    created_at: createdAt,
    error: null,
    id: responseId,
    incomplete_details: null,
    instructions: null,
    max_output_tokens: null,
    metadata: {},
    model: model ?? "gpt-5-codex",
    object: "response",
    output: [output],
    parallel_tool_calls: true,
    previous_response_id: null,
    reasoning: { effort: null, summary: null },
    status: "completed",
    store: false,
    temperature: 1,
    text: { format: { type: "text" } },
    tool_choice: "auto",
    tools: [],
    top_p: 1,
    truncation: "disabled",
    usage: {
      input_tokens: 1,
      input_tokens_details: { cached_tokens: 0 },
      output_tokens: 3,
      output_tokens_details: { reasoning_tokens: 0 },
      total_tokens: 4,
    },
    user: null,
  };

  response.writeHead(200, sseHeaders());
  writeSse(response, "response.created", {
    response: { ...completed, output: [], status: "in_progress" },
    type: "response.created",
  });
  writeSse(response, "response.in_progress", {
    response: { ...completed, output: [], status: "in_progress" },
    type: "response.in_progress",
  });
  writeSse(response, "response.output_item.added", {
    item: { content: [], id: itemId, role: "assistant", status: "in_progress", type: "message" },
    output_index: 0,
    type: "response.output_item.added",
  });
  writeSse(response, "response.content_part.added", {
    content_index: 0,
    item_id: itemId,
    output_index: 0,
    part: { annotations: [], text: "", type: "output_text" },
    type: "response.content_part.added",
  });
  writeSse(response, "response.output_text.delta", {
    content_index: 0,
    delta: openAiText,
    item_id: itemId,
    output_index: 0,
    type: "response.output_text.delta",
  });
  writeSse(response, "response.output_text.done", {
    content_index: 0,
    item_id: itemId,
    output_index: 0,
    text: openAiText,
    type: "response.output_text.done",
  });
  writeSse(response, "response.content_part.done", {
    content_index: 0,
    item_id: itemId,
    output_index: 0,
    part: output.content[0],
    type: "response.content_part.done",
  });
  writeSse(response, "response.output_item.done", {
    item: output,
    output_index: 0,
    type: "response.output_item.done",
  });
  writeSse(response, "response.completed", {
    response: completed,
    type: "response.completed",
  });
  response.end();
}

function sendAnthropicStream(response, model) {
  response.writeHead(200, sseHeaders());
  writeSse(response, "message_start", {
    message: {
      content: [],
      id: "msg_cutokyo_mock",
      model: model ?? "claude-3-5-haiku-latest",
      role: "assistant",
      stop_reason: null,
      stop_sequence: null,
      type: "message",
      usage: { input_tokens: 1, output_tokens: 0 },
    },
    type: "message_start",
  });
  writeSse(response, "content_block_start", {
    content_block: { text: "", type: "text" },
    index: 0,
    type: "content_block_start",
  });
  writeSse(response, "content_block_delta", {
    delta: { text: anthropicText, type: "text_delta" },
    index: 0,
    type: "content_block_delta",
  });
  writeSse(response, "content_block_stop", { index: 0, type: "content_block_stop" });
  writeSse(response, "message_delta", {
    delta: { stop_reason: "end_turn", stop_sequence: null },
    type: "message_delta",
    usage: { output_tokens: 3 },
  });
  writeSse(response, "message_stop", { type: "message_stop" });
  response.end();
}

function sendGeminiStream(response, model) {
  response.writeHead(200, sseHeaders());
  writeGeminiData(response, geminiResponse(model));
  response.end();
}

function geminiResponse(model) {
  return {
    candidates: [
      {
        content: {
          parts: [{ text: geminiText }],
          role: "model",
        },
        finishReason: "STOP",
        index: 0,
      },
    ],
    modelVersion: model ?? "gemini-3.1-flash-lite",
    responseId: "gemini_cutokyo_mock",
    usageMetadata: {
      candidatesTokenCount: 3,
      promptTokenCount: 1,
      totalTokenCount: 4,
    },
  };
}

function modelNameFromGeminiUrl(url) {
  const modelSegment = url.pathname.split("/").at(-1) ?? "gemini-3.1-flash-lite";
  return modelSegment.split(":").at(0);
}

function sseHeaders() {
  return {
    "cache-control": "no-cache",
    connection: "keep-alive",
    "content-type": "text/event-stream; charset=utf-8",
  };
}

function writeSse(response, event, data) {
  response.write(`event: ${event}\n`);
  response.write(`data: ${JSON.stringify(data)}\n\n`);
}

function writeGeminiData(response, data) {
  response.write(`data: ${JSON.stringify(data)}\n\n`);
}

function readBody(request) {
  return new Promise((resolve) => {
    const chunks = [];
    request.on("data", (chunk) => chunks.push(chunk));
    request.on("end", () => resolve(Buffer.concat(chunks).toString("utf8")));
  });
}

function writeJson(response, value, status = 200) {
  response.writeHead(status, { "content-type": "application/json" });
  response.end(JSON.stringify(value));
}

function parseJson(value) {
  try {
    return JSON.parse(value);
  } catch {
    return {};
  }
}

async function waitForJson(url) {
  const deadline = Date.now() + 20_000;
  let lastError = null;
  while (Date.now() < deadline) {
    try {
      return await getJson(url);
    } catch (error) {
      lastError = error;
      await delay(250);
    }
  }
  throw lastError ?? new Error(`Timed out waiting for ${url}`);
}

async function waitForExport(baseUrl) {
  let lastStatus = null;
  for (let attempt = 0; attempt < 80; attempt += 1) {
    lastStatus = await getJson(`${baseUrl}/cutokyo/export/status`);
    if (lastStatus.pendingEvents === 0 && lastStatus.lastSuccess) {
      return lastStatus;
    }
    await delay(250);
  }
  throw new Error(`OTLP export did not drain: ${JSON.stringify(lastStatus)}`);
}

async function getJson(url) {
  const response = await fetch(url);
  assert(response.ok, `${url} failed with ${response.status}.`);
  return response.json();
}

async function postJson(url, body) {
  const response = await fetch(url, {
    body: JSON.stringify(body),
    headers: { "content-type": "application/json" },
    method: "POST",
  });
  assert(response.ok, `${url} failed with ${response.status}.`);
  return response.json();
}

async function putJson(url, body) {
  const response = await fetch(url, {
    body: JSON.stringify(body),
    headers: { "content-type": "application/json" },
    method: "PUT",
  });
  assert(response.ok, `${url} failed with ${response.status}.`);
  return response.json();
}

async function postProviderRequest(url, body) {
  const response = await fetch(url, {
    body: JSON.stringify(body),
    headers: { "content-type": "application/json" },
    method: "POST",
  });
  assert(response.ok, `${url} failed with ${response.status}.`);
  return response.text();
}

async function readActivatedEnvValue(envPath, key) {
  const content = await readFile(envPath, "utf8");
  for (const line of content.split(/\r?\n/)) {
    const [name, ...valueParts] = line.split("=");
    if (name === key) {
      return valueParts.join("=").trim();
    }
  }
  throw new Error(`Activation did not write ${key} to ${envPath}.`);
}

function run(command, args, options = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: options.cwd,
      env: options.env,
      shell: process.platform === "win32",
      stdio: ["pipe", "pipe", "pipe"],
    });
    const stdout = [];
    const stderr = [];
    const timeout = options.timeoutMs
      ? setTimeout(() => {
          child.kill("SIGKILL");
          reject(new Error(`${command} ${args.join(" ")} timed out.`));
        }, options.timeoutMs)
      : null;

    child.stdout.on("data", (chunk) => stdout.push(chunk.toString()));
    child.stderr.on("data", (chunk) => stderr.push(chunk.toString()));
    child.on("error", (error) => {
      if (timeout) {
        clearTimeout(timeout);
      }
      reject(error);
    });
    child.on("exit", (code, signal) => {
      if (timeout) {
        clearTimeout(timeout);
      }
      const result = {
        code,
        signal,
        stderr: stderr.join(""),
        stdout: stdout.join(""),
      };
      if (code === 0) {
        resolve(result);
        return;
      }
      reject(
        new Error(`${command} ${args.join(" ")} failed with ${signal ?? code}\n${result.stderr}`),
      );
    });

    if (options.input) {
      child.stdin.end(options.input);
    } else {
      child.stdin.end();
    }
  });
}

function lastCapture(provider) {
  return captures.filter((captureEntry) => captureEntry.provider === provider).at(-1);
}

function countOccurrences(text, needle) {
  return text.split(needle).length - 1;
}

function parseClaudeResult(stdout) {
  try {
    return JSON.parse(stdout).result;
  } catch {
    return stdout.trim().split("\n").at(-1);
  }
}

function parseGeminiResult(stdout) {
  try {
    const parsed = JSON.parse(stdout);
    if (typeof parsed.result === "string") {
      return parsed.result;
    }
    if (typeof parsed.text === "string") {
      return parsed.text;
    }
    if (typeof parsed.response === "string") {
      return parsed.response;
    }
  } catch {
    // Fall through to the stable smoke marker below.
  }
  return stdout.includes(geminiText) ? geminiText : stdout.trim().split("\n").at(-1);
}

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}
