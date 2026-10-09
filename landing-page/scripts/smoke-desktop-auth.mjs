import { spawn } from "node:child_process";
import { access, mkdtemp, readFile, rm } from "node:fs/promises";
import http from "node:http";
import os from "node:os";
import path from "node:path";

const clientId = "client_cutokyo_smoke";
const proxyPort = await findFreePort();
const proxyBaseUrl = `http://127.0.0.1:${proxyPort}`;
const redirectUri = `${proxyBaseUrl}/cutokyo/auth/callback`;

const tokenRequests = [];
const mock = await startMockWorkos();
const authkitDomain = `http://127.0.0.1:${mock.port}`;
const tempHome = await mkdtemp(path.join(os.tmpdir(), "cutokyo-desktop-auth-"));

let proxy = null;

try {
  await run("cargo", ["build", "-p", "cutokyo-desktop", "--bin", "cutokyo"], {
    cwd: "..",
    env: {
      ...process.env,
      CUTOKYO_WORKOS_AUTHKIT_DOMAIN: authkitDomain,
      CUTOKYO_WORKOS_CLIENT_ID: clientId,
      CUTOKYO_WORKOS_REDIRECT_URI: redirectUri,
    },
  });

  proxy = spawn(binaryPath(), [], {
    cwd: "..",
    env: proxyEnv(tempHome, proxyPort),
    stdio: ["ignore", "pipe", "pipe"],
  });

  const stderr = [];
  proxy.stderr.on("data", (chunk) => stderr.push(chunk.toString()));

  const status = await waitForJson(`${proxyBaseUrl}/cutokyo/status`);
  assert(status.status === "ready", "Cutokyo proxy did not report ready.");
  assert(status.auth?.desktopConfigured === true, "Desktop auth was not configured.");

  const runtimeWorkosEnvCount = Object.keys(proxyEnv(tempHome, proxyPort)).filter((key) =>
    key.startsWith("CUTOKYO_WORKOS_"),
  ).length;
  assert(runtimeWorkosEnvCount === 0, "Proxy runtime env still contained WorkOS config.");

  const start = await postJson(`${proxyBaseUrl}/cutokyo/auth/start`);
  const authorizationUrl = new URL(start.authorizationUrl);
  assert(authorizationUrl.origin === authkitDomain, "Authorization URL did not use mock WorkOS.");
  assert(authorizationUrl.pathname === "/oauth2/authorize", "Unexpected authorize path.");
  assert(authorizationUrl.searchParams.get("client_id") === clientId, "Client ID mismatch.");
  assert(authorizationUrl.searchParams.get("redirect_uri") === redirectUri, "Redirect mismatch.");
  assert(authorizationUrl.searchParams.get("code_challenge"), "Missing PKCE challenge.");
  const state = authorizationUrl.searchParams.get("state");
  assert(state, "Missing OAuth state.");

  const callback = await fetch(
    `${proxyBaseUrl}/cutokyo/auth/callback?code=workos_smoke_code&state=${encodeURIComponent(
      state,
    )}`,
  );
  assert(callback.ok, `Callback failed with ${callback.status}.`);
  const callbackHtml = await callback.text();
  assert(
    callbackHtml.includes("Cutokyo is signed in"),
    `Callback did not render success: ${callbackHtml}`,
  );

  const session = await getJson(`${proxyBaseUrl}/cutokyo/auth/session`);
  assert(session.authenticated === true, "Session did not become authenticated.");
  assert(session.authConfigured === true, "Session did not report auth configured.");
  assert(session.user?.email === "cutokyo@example.com", "Session user email mismatch.");
  assert(!session.storageError, `OS credential storage failed: ${session.storageError}`);
  await assertMissing(path.join(tempHome, ".cutokyo", "auth-session.json"));
  const encryptedSession = await readFile(
    path.join(tempHome, ".cutokyo", "auth-session.enc"),
    "utf8",
  );
  assert(encryptedSession.includes("XChaCha20-Poly1305"), "Session fallback is not encrypted.");
  assert(
    !encryptedSession.includes("access_cutokyo_smoke"),
    "Encrypted session leaked access token.",
  );
  assert(
    !encryptedSession.includes("refresh_cutokyo_smoke"),
    "Encrypted session leaked refresh token.",
  );

  assert(tokenRequests.length === 1, "Expected one WorkOS token exchange.");
  const tokenRequest = tokenRequests[0];
  assert(tokenRequest.client_id === clientId, "Token exchange client_id mismatch.");
  assert(tokenRequest.code === "workos_smoke_code", "Token exchange code mismatch.");
  assert(tokenRequest.redirect_uri === redirectUri, "Token exchange redirect mismatch.");
  assert(tokenRequest.grant_type === "authorization_code", "Token grant type mismatch.");
  assert(
    typeof tokenRequest.code_verifier === "string" && tokenRequest.code_verifier.length >= 43,
    "Token exchange did not include a PKCE verifier.",
  );

  await postJson(`${proxyBaseUrl}/cutokyo/auth/sign-out`);
  const signedOut = await getJson(`${proxyBaseUrl}/cutokyo/auth/session`);
  assert(signedOut.authenticated === false, "Sign-out did not clear desktop session.");

  console.log(
    JSON.stringify(
      {
        ok: true,
        authkitDomain,
        runtimeWorkosEnvCount,
        authorizationHost: authorizationUrl.host,
        tokenRequests: tokenRequests.length,
        userEmail: session.user.email,
      },
      null,
      2,
    ),
  );

  if (stderr.length > 0) {
    console.error(stderr.join(""));
  }
} finally {
  if (proxy && !proxy.killed) {
    proxy.kill();
  }
  await mock.close();
  await rm(tempHome, { force: true, recursive: true });
}

function binaryPath() {
  const executable = process.platform === "win32" ? "cutokyo.exe" : "cutokyo";
  return path.join("target", "debug", executable);
}

function proxyEnv(home, port) {
  const env = Object.fromEntries(
    Object.entries(process.env).filter(([key]) => !key.startsWith("CUTOKYO_WORKOS_")),
  );
  return {
    ...env,
    CUTOKYO_API_ADDR: `127.0.0.1:${port}`,
    CUTOKYO_AUTH_SESSION_ENCRYPTION_KEY: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
    CUTOKYO_CONTROL_ONLY: "1",
    CUTOKYO_PROXY_ONLY: "1",
    CUTOKYO_SKIP_AUTO_CONNECT: "1",
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

async function assertMissing(filePath) {
  try {
    await access(filePath);
  } catch {
    return;
  }
  throw new Error(`Sensitive session file still exists: ${filePath}`);
}

function startMockWorkos() {
  const server = http.createServer(async (request, response) => {
    if (request.method === "POST" && request.url === "/oauth2/token") {
      const body = await readBody(request);
      const form = Object.fromEntries(new URLSearchParams(body));
      tokenRequests.push(form);
      writeJson(response, {
        access_token: "access_cutokyo_smoke",
        expires_in: 3600,
        id_token: makeIdToken(),
        refresh_token: "refresh_cutokyo_smoke",
      });
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

function makeIdToken() {
  const header = base64Url(JSON.stringify({ alg: "none", typ: "JWT" }));
  const payload = base64Url(
    JSON.stringify({
      email: "cutokyo@example.com",
      name: "Cutokyo Tester",
      sub: "user_cutokyo_smoke",
    }),
  );
  return `${header}.${payload}.`;
}

function base64Url(value) {
  return Buffer.from(value).toString("base64url");
}

function run(command, args, options) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      ...options,
      shell: process.platform === "win32",
      stdio: "inherit",
    });
    child.on("error", reject);
    child.on("exit", (code, signal) => {
      if (code === 0) {
        resolve();
        return;
      }
      reject(new Error(`${command} ${args.join(" ")} failed with ${signal ?? code}`));
    });
  });
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

async function getJson(url) {
  const response = await fetch(url);
  assert(response.ok, `${url} failed with ${response.status}.`);
  return response.json();
}

async function postJson(url) {
  const response = await fetch(url, { method: "POST" });
  assert(response.ok, `${url} failed with ${response.status}.`);
  return response.json();
}

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}
