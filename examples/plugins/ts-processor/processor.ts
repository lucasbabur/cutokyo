import { createInterface } from "node:readline";

type Capability = "transcript_read" | "network" | "emit_observation" | "emit_derived_fact";
type JsonObject = Record<string, unknown>;
type Envelope = {
  protocol_major: 1;
  kind: "handshake" | "request" | "response" | "cancel";
  request_id: string;
  payload: JsonObject;
};

const MAX_LINE_BYTES = 1024 * 1024;
const MAX_ITEMS = 1000;
const CAPABILITIES: Capability[] = ["emit_derived_fact"];
const CAPABILITY_VALUES = new Set<Capability>([
  "transcript_read",
  "network",
  "emit_observation",
  "emit_derived_fact",
]);
const IDENTIFIER = /^[A-Za-z0-9_.:/@-]+$/;
const REQUEST_ID = /^[A-Za-z0-9_.:@/-]+$/;

function fail(reason: string): never {
  throw new Error(reason);
}

function object(value: unknown, field: string): JsonObject {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return fail(`${field} must be an object`);
  }
  return value as JsonObject;
}

function exactKeys(value: JsonObject, allowed: string[], required: string[], field: string): void {
  const keys = Object.keys(value);
  if (keys.length > 64 || keys.some((key) => !allowed.includes(key)) || required.some((key) => !(key in value))) {
    fail(`${field} fields are invalid`);
  }
}

function boundedString(value: unknown, maximum: number, field: string): string {
  if (typeof value !== "string" || value.length === 0 || Buffer.byteLength(value, "utf8") > maximum) {
    return fail(`${field} is invalid`);
  }
  return value;
}

function identifier(value: unknown, field: string): string {
  const text = boundedString(value, 256, field);
  if (!IDENTIFIER.test(text)) {
    fail(`${field} is invalid`);
  }
  return text;
}

function requestId(value: unknown): string {
  const text = boundedString(value, 128, "request_id");
  if (!REQUEST_ID.test(text)) {
    fail("request_id is invalid");
  }
  return text;
}

function validateObjectBounds(value: unknown): void {
  if (Array.isArray(value)) {
    for (const item of value) validateObjectBounds(item);
    return;
  }
  if (typeof value === "object" && value !== null) {
    const fields = Object.entries(value as JsonObject);
    if (fields.length > 64) fail("nested object exceeds 64 fields");
    for (const [, nested] of fields) validateObjectBounds(nested);
  }
}

function capabilities(value: unknown, field: string): Capability[] {
  if (!Array.isArray(value) || value.length > 8) return fail(`${field} is invalid`);
  const result: Capability[] = [];
  for (const item of value) {
    if (typeof item !== "string" || !CAPABILITY_VALUES.has(item as Capability)) {
      return fail(`${field} contains an unknown capability`);
    }
    const capability = item as Capability;
    if (result.includes(capability)) fail(`${field} contains a duplicate`);
    result.push(capability);
  }
  return result;
}

function validateHandshake(payload: JsonObject): void {
  exactKeys(payload, ["plugin_id", "plugin_kind", "capabilities", "cursor_idempotent"], ["plugin_id", "plugin_kind", "capabilities", "cursor_idempotent"], "handshake payload");
  identifier(payload.plugin_id, "plugin_id");
  if (payload.plugin_kind !== "processor" || payload.cursor_idempotent !== false) fail("handshake identity is invalid");
  const declared = capabilities(payload.capabilities, "capabilities");
  if (declared.length !== 1 || declared[0] !== "emit_derived_fact") fail("handshake capabilities are invalid");
}

function validateProcessorRequest(payload: JsonObject): void {
  exactKeys(payload, ["operation", "granted_capabilities", "records", "transcripts"], ["operation", "granted_capabilities", "records"], "request payload");
  if (payload.operation !== "process_records") fail("operation must be process_records");
  const granted = capabilities(payload.granted_capabilities, "granted_capabilities");
  if (!granted.includes("emit_derived_fact")) fail("emit_derived_fact was not granted");
  if (!Array.isArray(payload.records) || payload.records.length > MAX_ITEMS) fail("records are invalid");
  for (const recordValue of payload.records) {
    const record = object(recordValue, "record");
    const count = Object.keys(record).length;
    if (count === 0 || count > 64) fail("record fields are invalid");
    validateObjectBounds(record);
  }
  if (payload.transcripts !== undefined) {
    if (!granted.includes("transcript_read") || !Array.isArray(payload.transcripts) || payload.transcripts.length > MAX_ITEMS) {
      fail("transcripts are invalid or ungranted");
    }
    for (const transcript of payload.transcripts) {
      if (typeof transcript !== "string" || Buffer.byteLength(transcript, "utf8") > MAX_LINE_BYTES) fail("transcript is invalid");
    }
  }
}

function validateTelemetry(value: unknown): void {
  const telemetry = object(value, "telemetry");
  exactKeys(telemetry, ["name", "value", "unit"], ["name", "value"], "telemetry");
  const name = boundedString(telemetry.name, 128, "telemetry.name");
  if (!/^[A-Za-z0-9_.:-]+$/.test(name)) fail("telemetry.name is invalid");
  if (!Number.isSafeInteger(telemetry.value) || (telemetry.value as number) < 0) fail("telemetry.value is invalid");
  if (telemetry.unit !== undefined) boundedString(telemetry.unit, 32, "telemetry.unit");
}

function validateResponse(payload: JsonObject): void {
  exactKeys(payload, ["status", "cursor", "observations", "tags", "derived_facts", "used_capabilities", "telemetry"], ["status", "used_capabilities"], "response payload");
  if (payload.status !== "completed" && payload.status !== "cancelled") fail("response status is invalid");
  const used = capabilities(payload.used_capabilities, "used_capabilities");
  if (used.some((item) => item !== "emit_derived_fact")) fail("response used an undeclared capability");
  if (payload.cursor !== undefined && payload.cursor !== null && (typeof payload.cursor !== "string" || Buffer.byteLength(payload.cursor, "utf8") > 4096)) fail("response cursor is invalid");
  if (payload.observations !== undefined) fail("processor responses cannot emit observations");
  if (payload.tags !== undefined) {
    if (!Array.isArray(payload.tags) || payload.tags.length > MAX_ITEMS) fail("tags are invalid");
    for (const tag of payload.tags) boundedString(tag, 256, "tag");
  }
  if (payload.derived_facts !== undefined) {
    if (!Array.isArray(payload.derived_facts) || payload.derived_facts.length > MAX_ITEMS) fail("derived_facts are invalid");
    for (const factValue of payload.derived_facts) {
      const fact = object(factValue, "derived_fact");
      exactKeys(fact, ["fact_kind", "value", "source_observation_ids"], ["fact_kind", "value", "source_observation_ids"], "derived_fact");
      boundedString(fact.fact_kind, 128, "fact_kind");
      if (!Array.isArray(fact.source_observation_ids) || fact.source_observation_ids.length === 0 || fact.source_observation_ids.length > 128) fail("source_observation_ids are invalid");
      const seen = new Set<string>();
      for (const source of fact.source_observation_ids) {
        const id = identifier(source, "source_observation_id");
        if (seen.has(id)) fail("source_observation_ids contain a duplicate");
        seen.add(id);
      }
      validateObjectBounds(fact.value);
    }
  }
  if ((payload.tags as unknown[] | undefined)?.length || (payload.derived_facts as unknown[] | undefined)?.length) {
    if (!used.includes("emit_derived_fact")) fail("response omitted emit_derived_fact use");
  }
  if (payload.telemetry !== undefined) validateTelemetry(payload.telemetry);
}

function validateMessage(value: unknown, direction: "incoming" | "outgoing"): Envelope {
  const candidate = object(value, `${direction} envelope`);
  exactKeys(candidate, ["protocol_major", "kind", "request_id", "payload"], ["protocol_major", "kind", "request_id", "payload"], `${direction} envelope`);
  if (candidate.protocol_major !== 1) fail(`${direction} protocol_major must be 1`);
  const id = requestId(candidate.request_id);
  const payload = object(candidate.payload, `${direction} payload`);
  validateObjectBounds(payload);
  if (direction === "incoming") {
    if (candidate.kind === "request") validateProcessorRequest(payload);
    else if (candidate.kind === "cancel") exactKeys(payload, [], [], "cancel payload");
    else fail("incoming kind must be request or cancel");
  } else if (candidate.kind === "handshake") validateHandshake(payload);
  else if (candidate.kind === "response") validateResponse(payload);
  else fail("outgoing kind must be handshake or response");
  return { protocol_major: 1, kind: candidate.kind as Envelope["kind"], request_id: id, payload };
}

function send(message: unknown): void {
  const validated = validateMessage(message, "outgoing");
  const line = JSON.stringify(validated);
  if (Buffer.byteLength(line, "utf8") + 1 > MAX_LINE_BYTES) fail("outgoing line exceeds 1 MiB");
  process.stdout.write(`${line}\n`);
}

send({
  protocol_major: 1,
  kind: "handshake",
  request_id: "handshake",
  payload: {
    plugin_id: "example:ts-processor",
    plugin_kind: "processor",
    capabilities: CAPABILITIES,
    cursor_idempotent: false,
  },
});

const lines = createInterface({ input: process.stdin, crlfDelay: Infinity });
lines.on("line", (line: string) => {
  try {
    if (Buffer.byteLength(line, "utf8") + 1 > MAX_LINE_BYTES) fail("incoming line exceeds 1 MiB");
    const parsed = validateMessage(JSON.parse(line) as unknown, "incoming");
    if (parsed.kind === "cancel") {
      send({ protocol_major: 1, kind: "response", request_id: parsed.request_id, payload: { status: "cancelled", used_capabilities: [] } });
      return;
    }
    const records = parsed.payload.records as unknown[];
    const record = object(records[0], "first record");
    if (!Array.isArray(record.source_observation_ids) || typeof record.source_observation_ids[0] !== "string") fail("source observation attribution is missing");
    const sourceId = identifier(record.source_observation_ids[0], "source_observation_id");
    send({
      protocol_major: 1,
      kind: "response",
      request_id: parsed.request_id,
      payload: {
        status: "completed",
        used_capabilities: ["emit_derived_fact"],
        tags: ["example:verified"],
        derived_facts: [{ fact_kind: "example.processor_verified", value: true, source_observation_ids: [sourceId] }],
      },
    });
  } catch {
    process.exitCode = 2;
    lines.close();
  }
});
