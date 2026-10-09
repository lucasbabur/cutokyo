export function authkitProxy() {
  return () => Response.json({ status: "desktop-static-auth-stub" });
}

export function handleAuth() {
  return async () => Response.json({ status: "desktop-static-auth-stub" });
}

export async function getSignInUrl() {
  return "http://localhost:3000/";
}
