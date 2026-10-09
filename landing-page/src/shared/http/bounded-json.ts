export const MAX_JSON_RESPONSE_BYTES = 8 * 1024 * 1024;

type FetchBody = Pick<Request, "body" | "headers" | "text">;

export class JsonBodyTooLargeError extends Error {}

export async function readBoundedJson<T>(
  message: FetchBody,
  label: string,
  maxBytes = MAX_JSON_RESPONSE_BYTES,
): Promise<T> {
  const declaredLength = Number(message.headers.get("content-length"));
  if (Number.isFinite(declaredLength) && declaredLength > maxBytes) {
    throw new JsonBodyTooLargeError(`${label} exceeded ${maxBytes} bytes.`);
  }
  if (!message.body) return JSON.parse(await message.text()) as T;

  const reader = message.body.getReader();
  const chunks: Uint8Array[] = [];
  let totalBytes = 0;
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    if (value.byteLength > maxBytes - totalBytes) {
      await reader.cancel().catch(() => undefined);
      throw new JsonBodyTooLargeError(`${label} exceeded ${maxBytes} bytes.`);
    }
    chunks.push(value);
    totalBytes += value.byteLength;
  }

  const bytes = new Uint8Array(totalBytes);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return JSON.parse(new TextDecoder().decode(bytes)) as T;
}
