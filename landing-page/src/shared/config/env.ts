import "client-only";

import { createEnv } from "@t3-oss/env-nextjs";
import { z } from "zod";

export const publicHttpUrlSchema = z.url().refine((value) => {
  const protocol = new URL(value).protocol;
  return protocol === "http:" || protocol === "https:";
}, "URL must use HTTP or HTTPS");

export const publicHttpsUrlSchema = z
  .url()
  .refine((value) => new URL(value).protocol === "https:", "URL must use HTTPS");

export const env = createEnv({
  client: {
    NEXT_PUBLIC_APP_ENV: z.enum(["development", "test", "production"]).default("development"),
    NEXT_PUBLIC_APP_URL: publicHttpUrlSchema.default("http://localhost:3000"),
    NEXT_PUBLIC_FASTAPI_BASE_URL: publicHttpUrlSchema.default("http://localhost:8000"),
    NEXT_PUBLIC_FASTAPI_OPENAPI_URL: publicHttpUrlSchema.default(
      "http://localhost:8000/openapi.json",
    ),
    NEXT_PUBLIC_CUTOKYO_LOCAL_API_URL: publicHttpUrlSchema.default("http://localhost:49321"),
    NEXT_PUBLIC_CUTOKYO_TOKEN_SAVING: z.enum(["on", "off"]).default("off"),
    NEXT_PUBLIC_CUTOKYO_RELEASE_BASE_URL: publicHttpsUrlSchema.optional(),
    NEXT_PUBLIC_WORKOS_REDIRECT_URI: publicHttpUrlSchema.default(
      "http://localhost:3000/auth/callback",
    ),
  },
  runtimeEnv: {
    NEXT_PUBLIC_APP_ENV: process.env.NEXT_PUBLIC_APP_ENV,
    NEXT_PUBLIC_APP_URL: process.env.NEXT_PUBLIC_APP_URL,
    NEXT_PUBLIC_FASTAPI_BASE_URL: process.env.NEXT_PUBLIC_FASTAPI_BASE_URL,
    NEXT_PUBLIC_FASTAPI_OPENAPI_URL: process.env.NEXT_PUBLIC_FASTAPI_OPENAPI_URL,
    NEXT_PUBLIC_CUTOKYO_LOCAL_API_URL: process.env.NEXT_PUBLIC_CUTOKYO_LOCAL_API_URL,
    NEXT_PUBLIC_CUTOKYO_TOKEN_SAVING: process.env.NEXT_PUBLIC_CUTOKYO_TOKEN_SAVING,
    NEXT_PUBLIC_CUTOKYO_RELEASE_BASE_URL: process.env.NEXT_PUBLIC_CUTOKYO_RELEASE_BASE_URL,
    NEXT_PUBLIC_WORKOS_REDIRECT_URI: process.env.NEXT_PUBLIC_WORKOS_REDIRECT_URI,
  },
});
