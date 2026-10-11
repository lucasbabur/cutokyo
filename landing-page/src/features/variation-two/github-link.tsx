"use client";

import { useEffect, useState } from "react";

import { getCutokyoGitHubStars } from "@/infrastructure/github";

import styles from "./view.module.css";

import type { VariationLocale } from "./i18n";

const repositoryUrl = "https://github.com/lucasbabur/cutokyo";
const refreshIntervalMs = 120_000;
const starsCacheKey = "cutokyo:github-stars";
// Confirmed on the public repository page on 2026-10-10. Used until a live count arrives.
const lastVerifiedStars = 1;

export function GitHubLink({ locale }: { locale: VariationLocale }) {
  const [stars, setStars] = useState(lastVerifiedStars);

  useEffect(() => {
    const controller = new AbortController();
    let pending = false;

    queueMicrotask(() => {
      if (controller.signal.aborted) return;
      try {
        const cached: unknown = JSON.parse(localStorage.getItem(starsCacheKey) ?? "null");
        if (typeof cached === "number" && Number.isSafeInteger(cached) && cached >= 0) {
          setStars(cached);
        }
      } catch {
        // Storage may be unavailable; the verified fallback remains visible.
      }
    });

    const refresh = () => {
      if (document.hidden || pending || controller.signal.aborted) return;
      pending = true;

      void getCutokyoGitHubStars(controller.signal)
        .then((count) => {
          if (controller.signal.aborted) return;
          setStars(count);
          try {
            localStorage.setItem(starsCacheKey, JSON.stringify(count));
          } catch {
            // A storage restriction must not prevent live updates.
          }
        })
        .catch(() => {
          // Keep the last confirmed count during temporary GitHub/network failures.
        })
        .finally(() => {
          pending = false;
        });
    };

    refresh();
    const timer = window.setInterval(refresh, refreshIntervalMs);
    window.addEventListener("focus", refresh);
    window.addEventListener("online", refresh);
    window.addEventListener("pageshow", refresh);
    document.addEventListener("visibilitychange", refresh);

    return () => {
      controller.abort();
      window.clearInterval(timer);
      window.removeEventListener("focus", refresh);
      window.removeEventListener("online", refresh);
      window.removeEventListener("pageshow", refresh);
      document.removeEventListener("visibilitychange", refresh);
    };
  }, []);

  const starLabel = locale === "pt-BR" ? "estrelas" : "stars";
  const label = `${locale === "pt-BR" ? "Ver Cutokyo no GitHub" : "View Cutokyo on GitHub"} · ${stars} ${starLabel}`;

  return (
    <a
      aria-label={label}
      className={styles.githubLink}
      href={repositoryUrl}
      rel="noopener noreferrer"
      target="_blank"
    >
      <svg aria-hidden="true" className={styles.githubIcon} fill="currentColor" viewBox="0 0 16 16">
        <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82A7.64 7.64 0 0 1 8 3.87c.68 0 1.36.09 2 .26 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.01 8.01 0 0 0 16 8c0-4.42-3.58-8-8-8Z" />
      </svg>
      <b aria-hidden="true" className={styles.githubStars} data-stars>
        {new Intl.NumberFormat("en-US").format(stars)}
      </b>
    </a>
  );
}
