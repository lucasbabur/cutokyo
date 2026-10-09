export function isWorkosConfigured(): boolean {
  if (isE2eAuthkitConfigured()) return true;
  return Boolean(
    readEnv("WORKOS_API_KEY") &&
    readEnv("WORKOS_CLIENT_ID") &&
    readEnv("WORKOS_COOKIE_PASSWORD").length >= 32 &&
    readEnv("NEXT_PUBLIC_WORKOS_REDIRECT_URI"),
  );
}

function isE2eAuthkitConfigured(): boolean {
  if (readEnv("CUTOKYO_E2E_AUTHKIT") !== "1") return false;
  if (process.env.NODE_ENV === "production") {
    throw new Error("CUTOKYO_E2E_AUTHKIT is production-impossible");
  }
  if (readEnv("NEXT_PUBLIC_CUTOKYO_E2E_BEARER_TOKEN").length < 32) {
    throw new Error("NEXT_PUBLIC_CUTOKYO_E2E_BEARER_TOKEN must contain at least 32 characters");
  }
  try {
    const appUrl = new URL(readEnv("NEXT_PUBLIC_APP_URL"));
    return appUrl.protocol === "http:" && ["127.0.0.1", "localhost"].includes(appUrl.hostname);
  } catch {
    throw new Error("CUTOKYO_E2E_AUTHKIT requires a loopback NEXT_PUBLIC_APP_URL");
  }
}

function readEnv(name: string): string {
  return process.env[name]?.trim() ?? "";
}
