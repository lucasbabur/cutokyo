import "./globals.css";
import "@radix-ui/themes/styles.css";
import "@workos-inc/widgets/styles.css";

import { AuthKitProvider } from "@workos-inc/authkit-nextjs/components";

import { isWorkosConfigured } from "@/infrastructure/workos/config";

import { AppProviders } from "./providers";

import type { Metadata } from "next";

export const metadata: Metadata = {
  description:
    "Cutokyo adds governance, security redaction, and context analysis to LLM inputs while reducing token cost and latency.",
  icons: {
    apple: [{ url: "/apple-touch-icon.png", sizes: "180x180", type: "image/png" }],
    icon: [
      { url: "/favicon.ico", sizes: "any" },
      { url: "/brand/cutokyo-mark.svg", type: "image/svg+xml" },
      { url: "/favicon-32x32.png", sizes: "32x32", type: "image/png" },
      { url: "/favicon-16x16.png", sizes: "16x16", type: "image/png" },
    ],
  },
  manifest: "/site.webmanifest",
  title: "Cutokyo - Govern, Redact, and Cut Tokens",
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  const content = <AppProviders>{children}</AppProviders>;
  return (
    <html className="dark" lang="en">
      <body className="bg-background text-foreground antialiased">
        {isWorkosConfigured() ? <AuthKitProvider>{content}</AuthKitProvider> : content}
      </body>
    </html>
  );
}
