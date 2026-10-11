"use client";

/* eslint-disable @next/next/no-img-element -- The integrations use local SVG brand assets. */

import { useEffect, useRef, useState } from "react";

import { GitHubLink } from "./github-link";
import { HeroBackdrop, HeroFlow } from "./hero-flow";
import { variationCopies } from "./i18n";
import { InventoryDemo } from "./inventory-demo";
import styles from "./view.module.css";
import { WorkflowFlow } from "./workflow-flow";

import type { ProcessIconName, VariationCopy, VariationLocale } from "./i18n";
import type { CSSProperties, FormEvent, MouseEvent, ReactNode } from "react";

const supportedAgents = [
  { name: "Codex", src: "/logos/codex.svg", wordmark: null },
  { name: "Claude", src: null, wordmark: "/logos/claude-wordmark.svg" },
  { name: "OpenCode", src: null, wordmark: "/logos/opencode-wordmark.svg" },
] as const;

const processImages = [
  "/process/connect-agents.webp",
  "/process/work-as-usual.webp",
  "/process/resume-anywhere.webp",
] as const;

const capabilityImages = [
  "/capabilities/unified-memory.webp",
  "/capabilities/local-security.webp",
  "/capabilities/agent-observability.webp",
  "/capabilities/integration-control.webp",
] as const;

const desktopDownloads = [
  { name: "Windows", fileName: "Cutokyo-Windows-Setup.exe" },
  { name: "macOS Apple Silicon", fileName: "Cutokyo-macOS-arm64.dmg" },
  { name: "macOS Intel", fileName: "Cutokyo-macOS-x64.dmg" },
  { name: "Linux AppImage", fileName: "Cutokyo-Linux.AppImage" },
  { name: "Linux DEB", fileName: "Cutokyo-Linux.deb" },
] as const;

const desktopReleaseBaseUrl = (
  process.env.NEXT_PUBLIC_CUTOKYO_RELEASE_BASE_URL ??
  "https://github.com/lucasbabur/cutokyo/releases/latest/download"
).replace(/\/+$/, "");

const NAV_SCROLL_OFFSET = 96;
const SCROLL_DURATION_MS = 900;

const designCopies = {
  en: {
    stepsKicker: "No workflow migration",
    stepsTitle: "Keep your tools. Add a shared memory.",
    stepsBody:
      "Cutokyo connects through hooks whenever possible and quietly records the useful parts of every session locally.",
    productKicker: "Unified observability",
    productTitle: "Understand every session.",
    productBody:
      "See tokens, tools, models and context across every agent, with cost reduction as one part of the full picture.",
    finalKicker: "Your agents. One operational layer.",
    finalTitle: "Keep the tools. Gain the control plane.",
    finalBody: "Bring memory, security and observability to every AI coding session.",
    downloadsTitle: "Download Cutokyo",
    downloadAction: "Download",
  },
  "pt-BR": {
    stepsKicker: "Sem migrar seu fluxo",
    stepsTitle: "Mantenha suas ferramentas. Adicione uma memória compartilhada.",
    stepsBody:
      "A Cutokyo se conecta por hooks sempre que possível e registra localmente as partes úteis de cada sessão.",
    productKicker: "Observabilidade unificada",
    productTitle: "Entenda cada sessão.",
    productBody:
      "Veja tokens, ferramentas, modelos e contexto entre todos os agentes, com redução de custos como parte do cenário completo.",
    finalKicker: "Seus agentes. Uma camada operacional.",
    finalTitle: "Mantenha as ferramentas. Ganhe o painel de controle.",
    finalBody: "Leve memória, segurança e observabilidade para cada sessão de programação com IA.",
    downloadsTitle: "Baixe a Cutokyo",
    downloadAction: "Download",
  },
} as const;

export function VariationTwoView({ locale = "en" }: { locale?: VariationLocale }) {
  const copy: VariationCopy = variationCopies[locale];
  const designCopy = designCopies[locale];
  const [isModalOpen, setIsModalOpen] = useState(false);
  const [selectedCapabilityIndex, setSelectedCapabilityIndex] = useState(0);
  const selectedCapability = copy.cards[selectedCapabilityIndex] ?? (["diff", "", ""] as const);
  useReveal();
  useReportMotion();

  const handleSectionLinkClick = (event: MouseEvent<HTMLAnchorElement>) => {
    const hash = event.currentTarget.hash;
    const target = hash ? document.getElementById(hash.slice(1)) : null;
    if (!target) return;

    event.preventDefault();
    const navHeight = document.querySelector("nav")?.getBoundingClientRect().height ?? 0;
    const navOffset = hash === "#top" ? 0 : Math.max(NAV_SCROLL_OFFSET, navHeight + 12);
    const top = Math.max(0, window.scrollY + target.getBoundingClientRect().top - navOffset);
    animateScrollTo(top);
    window.history.pushState(null, "", hash);
  };

  useEffect(() => {
    document.body.style.overflow = isModalOpen ? "hidden" : "";
    return () => {
      document.body.style.overflow = "";
    };
  }, [isModalOpen]);

  return (
    <main className={styles.page}>
      <nav className={styles.nav}>
        <a className={styles.brand} href="#top" onClick={handleSectionLinkClick}>
          <span aria-hidden="true" className={styles.brandMark} />
          <span className={styles.navText}>{copy.brand}</span>
        </a>
        <div className={styles.navLinks}>
          <a href="#proof" onClick={handleSectionLinkClick}>
            <span className={styles.navText}>{copy.nav.trusted}</span>
          </a>
          <a href="#how" onClick={handleSectionLinkClick}>
            <span className={styles.navText}>{copy.nav.setup}</span>
          </a>
          <a href="#inside" onClick={handleSectionLinkClick}>
            <span className={styles.navText}>{copy.nav.inside}</span>
          </a>
        </div>
        <div className={styles.navActions}>
          <GitHubLink locale={locale} />
        </div>
      </nav>

      <section className={styles.hero} id="top">
        <HeroBackdrop />
        <HeroFlow locale={locale} />
        <div className={styles.heroText}>
          <div className={styles.heroCopy} data-reveal data-visible>
            <h1>{copy.hero.title}</h1>
          </div>
          <div className={styles.heroDetails} data-reveal data-visible>
            <p>{copy.hero.body}</p>
          </div>
        </div>
      </section>

      <section className={styles.logoBand} id="proof" aria-label={copy.nav.trusted}>
        <div className={styles.logoGrid}>
          {supportedAgents.map((agent) => (
            <div className={styles.logoTile} key={agent.name}>
              {agent.wordmark ? (
                <img alt={agent.name} className={styles.agentWordmark} src={agent.wordmark} />
              ) : (
                <>
                  {agent.src ? <img alt="" src={agent.src} /> : null}
                  <span className={styles.codexWordmark}>{agent.name}</span>
                </>
              )}
            </div>
          ))}
        </div>
      </section>

      <section className={styles.steps} id="how" aria-label={copy.nav.setup}>
        <WorkflowFlow />
        <div className={styles.stepsIntro} data-reveal>
          <p className={styles.kicker}>{designCopy.stepsKicker}</p>
          <h2>{designCopy.stepsTitle}</h2>
          <p>{designCopy.stepsBody}</p>
        </div>
        <div className={styles.stepsTrack} aria-label={designCopy.stepsTitle} tabIndex={0}>
          {copy.steps.map((step, index) => (
            <article
              className={[styles.processCard, step.percent ? styles.savingsCard : ""].join(" ")}
              tabIndex={0}
              data-reveal
              key={step.title}
              style={
                {
                  "--process-image": `url("${processImages[index]}")`,
                  "--reveal-delay": `${index * 180}ms`,
                } as CSSProperties
              }
            >
              <div className={styles.processCardCopy}>
                <div className={styles.stepTop}>
                  <span className={styles.stepNumber}>{step.num}</span>
                  <ProcessIcon name={step.icon} />
                </div>
                <h3>
                  {step.percent ? (
                    <>
                      {step.title} <AnimatedPercent delayMs={580} value={step.percent} />{" "}
                      <span className={styles.stepSuffix}>{step.suffix}</span>
                    </>
                  ) : (
                    step.title
                  )}
                </h3>
                <p>{step.body}</p>
              </div>
            </article>
          ))}
        </div>
      </section>

      <ContextControlPlane copy={copy} />

      <section className={styles.inside} id="inside">
        <div className={styles.sectionTitle} data-reveal>
          <p className={styles.kicker}>{copy.inside.kicker}</p>
          <h2>{copy.inside.title}</h2>
        </div>
        <div className={styles.capabilityExplorer} data-reveal>
          <div className={styles.capabilityList} aria-label={copy.inside.title}>
            {copy.cards.map(([icon, title], index) => {
              const isSelected = selectedCapabilityIndex === index;

              return (
                <button
                  aria-controls="capability-detail"
                  aria-pressed={isSelected}
                  className={styles.capabilityOption}
                  data-selected={isSelected}
                  key={title}
                  onClick={() => setSelectedCapabilityIndex(index)}
                  style={
                    {
                      "--capability-image": `url("${capabilityImages[index]}")`,
                    } as CSSProperties
                  }
                  type="button"
                >
                  <span className={styles.capabilityOptionNumber}>0{index + 1}</span>
                  <Icon name={icon} />
                  <span className={styles.capabilityOptionTitle}>{title}</span>
                  <span className={styles.capabilityOptionArrow} aria-hidden="true">
                    →
                  </span>
                </button>
              );
            })}
          </div>
          <article
            aria-live="polite"
            className={styles.capabilityDetail}
            id="capability-detail"
            style={
              {
                "--capability-image": `url("${capabilityImages[selectedCapabilityIndex]}")`,
              } as CSSProperties
            }
          >
            <span
              aria-hidden="true"
              className={styles.capabilityDetailTransition}
              key={`capability-transition-${selectedCapabilityIndex}`}
            />
            <div
              className={styles.capabilityDetailCopy}
              key={`capability-copy-${selectedCapabilityIndex}`}
            >
              <span className={styles.capabilityDetailMeta}>
                {`0${selectedCapabilityIndex + 1}`}
              </span>
              <h3>{selectedCapability[1]}</h3>
              <p>{selectedCapability[2]}</p>
            </div>
          </article>
        </div>
      </section>

      <section className={styles.downloads} id="downloads">
        <header className={styles.downloadsIntro} data-reveal>
          <h2>
            <span aria-hidden="true" className={styles.brandMark} />
            <span>{designCopy.downloadsTitle}</span>
          </h2>
        </header>
        <div className={styles.downloadList}>
          {desktopDownloads.map((download, index) => {
            const fileUrl = `${desktopReleaseBaseUrl}/${download.fileName}`;

            return (
              <article
                className={styles.downloadRow}
                data-reveal
                key={download.fileName}
                style={{ "--reveal-delay": `${index * 90}ms` } as CSSProperties}
              >
                <span className={styles.downloadNumber}>{String(index + 1).padStart(2, "0")}</span>
                <div className={styles.downloadName}>
                  <h3>{download.name}</h3>
                  <code>{download.fileName}</code>
                </div>
                <div className={styles.downloadActions}>
                  <a download href={fileUrl}>
                    <span>{designCopy.downloadAction}</span>
                    <svg aria-hidden="true" fill="none" viewBox="0 0 16 16">
                      <path
                        d="M8 2v8m-3-3 3 3 3-3M3 11v3h10v-3"
                        stroke="currentColor"
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        strokeWidth="1.5"
                      />
                    </svg>
                  </a>
                  <a download href={`${fileUrl}.sha256`}>
                    SHA-256
                  </a>
                </div>
              </article>
            );
          })}
        </div>
      </section>

      <section className={styles.finalCta}>
        <div data-reveal>
          <p className={styles.kicker}>{designCopy.finalKicker}</p>
          <h2>{designCopy.finalTitle}</h2>
          <p>{designCopy.finalBody}</p>
        </div>
        <button data-reveal onClick={() => setIsModalOpen(true)} type="button">
          Enterprise
          <span aria-hidden="true">↗</span>
        </button>
      </section>

      <footer className={styles.footer}>
        <div className={styles.footerBrand} data-reveal>
          <a href="#top" onClick={handleSectionLinkClick}>
            <span aria-hidden="true" className={styles.brandMark} />
            <span>{copy.brand}</span>
          </a>
        </div>
        <div className={styles.footerLinks} data-reveal>
          <a href="#proof" onClick={handleSectionLinkClick}>
            {copy.nav.trusted}
          </a>
          <a href="#how" onClick={handleSectionLinkClick}>
            {copy.nav.setup}
          </a>
          <a href="#inside" onClick={handleSectionLinkClick}>
            {copy.nav.inside}
          </a>
        </div>
        <p className={styles.footerMeta}>{copy.footer.copyright}</p>
      </footer>

      {isModalOpen ? <DemoModal copy={copy.modal} onClose={() => setIsModalOpen(false)} /> : null}
    </main>
  );
}

function animateScrollTo(top: number) {
  const start = window.scrollY;
  const distance = top - start;
  const startTime = performance.now();
  const easeOutCubic = (progress: number) => 1 - (1 - progress) ** 3;

  const tick = (now: number) => {
    const progress = Math.min(1, (now - startTime) / SCROLL_DURATION_MS);
    window.scrollTo(0, start + distance * easeOutCubic(progress));
    if (progress < 1) requestAnimationFrame(tick);
  };

  requestAnimationFrame(tick);
}

function ContextControlPlane({ copy }: { copy: VariationCopy }) {
  return (
    <section className={styles.controlPlane} id="governance" aria-labelledby="control-plane-title">
      <video
        aria-hidden="true"
        autoPlay
        className={styles.governanceBackdropVideo}
        loop
        muted
        playsInline
        poster="/video/governance-waves-yellow-hq-poster.webp"
        preload="metadata"
      >
        <source
          media="(max-width: 760px)"
          src="/video/governance-waves-yellow-mobile-tall-hq.mp4"
          type="video/mp4"
        />
        <source src="/video/governance-waves-yellow-hq.mp4" type="video/mp4" />
      </video>
      <header
        className={[styles.governanceIntro, styles.reportMotion].join(" ")}
        data-motion-delay="0"
      >
        <div className={styles.governanceIntroContent}>
          <p className={styles.kicker}>{copy.controlPlane.badge}</p>
          <h2 id="control-plane-title">{copy.controlPlane.title}</h2>
        </div>
      </header>

      <div className={styles.reportMotion} data-motion-delay="100">
        <InventoryDemo />
      </div>
    </section>
  );
}

function useReveal() {
  useEffect(() => {
    const els = Array.from(document.querySelectorAll<HTMLElement>("[data-reveal]"));
    if (!els.length) return;
    if (typeof IntersectionObserver === "undefined") {
      els.forEach((el) => el.setAttribute("data-visible", ""));
      return;
    }
    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting) {
            entry.target.setAttribute("data-visible", "");
            observer.unobserve(entry.target);
          }
        }
      },
      { rootMargin: "0px 0px -10% 0px", threshold: 0.14 },
    );
    els.forEach((el) => observer.observe(el));
    return () => observer.disconnect();
  }, []);
}

function useReportMotion() {
  useEffect(() => {
    if (typeof document === "undefined") return;
    const motionElements = Array.from(
      document.querySelectorAll<HTMLElement>(`.${styles.reportMotion}`),
    );
    const meterElements = Array.from(
      document.querySelectorAll<HTMLElement>(`.${styles.reportMeter}`),
    );
    const elements = [...motionElements, ...meterElements];
    if (!elements.length) return;

    elements.forEach((element) => {
      const delay = Math.max(0, Number(element.dataset.motionDelay ?? "0"));
      element.style.setProperty("--motion-delay", `${delay}ms`);
      element.dataset.motionPending = "true";
    });

    const shouldReduceMotion =
      typeof window.matchMedia === "function" &&
      window.matchMedia("(prefers-reduced-motion: reduce)").matches;

    if (shouldReduceMotion || typeof IntersectionObserver === "undefined") {
      elements.forEach((element) => {
        element.dataset.motionVisible = "true";
        delete element.dataset.motionPending;
      });
      return;
    }

    const revealElement = (element: HTMLElement) => {
      element.dataset.motionVisible = "true";
      delete element.dataset.motionPending;
    };

    const observer = new IntersectionObserver(
      (entries) => {
        entries.forEach((entry) => {
          if (!entry.isIntersecting) return;
          const element = entry.target as HTMLElement;
          revealElement(element);
          observer.unobserve(element);
        });
      },
      { rootMargin: "0px 0px -18% 0px", threshold: 0.16 },
    );

    elements.forEach((element) => observer.observe(element));

    const revealVisibleElements = () => {
      const viewportHeight = window.innerHeight || document.documentElement.clientHeight || 1;
      elements.forEach((element) => {
        if (element.dataset.motionVisible === "true") return;
        const rect = element.getBoundingClientRect();
        if (rect.top < viewportHeight * 0.88 && rect.bottom > viewportHeight * 0.08) {
          revealElement(element);
          observer.unobserve(element);
        }
      });
    };

    const frame = requestAnimationFrame(revealVisibleElements);

    return () => {
      cancelAnimationFrame(frame);
      observer.disconnect();
    };
  }, []);
}

function AnimatedPercent({ delayMs, value }: { delayMs: number; value: number }) {
  const ref = useRef<HTMLSpanElement>(null);
  const [displayValue, setDisplayValue] = useState(0);

  useEffect(() => {
    const element = ref.current;
    if (!element) return;

    let frame = 0;
    let timeout = 0;
    const duration = 900;

    const run = () => {
      const start = performance.now();
      const tick = (now: number) => {
        const progress = Math.min(1, (now - start) / duration);
        const eased = 1 - Math.pow(1 - progress, 3);
        setDisplayValue(Math.round(value * eased));
        if (progress < 1) frame = requestAnimationFrame(tick);
      };
      frame = requestAnimationFrame(tick);
    };

    if (typeof IntersectionObserver === "undefined") {
      run();
      return () => {
        if (frame) cancelAnimationFrame(frame);
      };
    }

    const observer = new IntersectionObserver(
      ([entry]) => {
        if (!entry?.isIntersecting) return;
        timeout = window.setTimeout(run, delayMs);
        observer.disconnect();
      },
      { threshold: 0.65 },
    );

    observer.observe(element);

    return () => {
      observer.disconnect();
      if (frame) cancelAnimationFrame(frame);
      if (timeout) window.clearTimeout(timeout);
    };
  }, [delayMs, value]);

  return (
    <span aria-label={`${value}%`} className={styles.percentValue} ref={ref}>
      {displayValue}%
    </span>
  );
}

function DemoModal({ copy, onClose }: { copy: VariationCopy["modal"]; onClose: () => void }) {
  const [isSubmitted, setIsSubmitted] = useState(false);

  const handleSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setIsSubmitted(true);
  };

  return (
    <div className={styles.modalBackdrop} onMouseDown={onClose} role="presentation">
      <section
        aria-labelledby="demo-title"
        aria-modal="true"
        className={styles.modal}
        onMouseDown={(event) => event.stopPropagation()}
        role="dialog"
      >
        <button
          aria-label={copy.closeLabel}
          className={styles.closeButton}
          onClick={onClose}
          type="button"
        >
          ×
        </button>
        <p className={styles.kicker}>{copy.kicker}</p>
        <h2 id="demo-title">{copy.title}</h2>
        <p className={styles.modalNote}>{copy.note}</p>
        {isSubmitted ? (
          <p className={styles.modalSuccess} role="status">
            {copy.success}
          </p>
        ) : (
          <form onSubmit={handleSubmit}>
            <label htmlFor="demo-email">{copy.emailLabel}</label>
            <input
              autoFocus
              id="demo-email"
              name="email"
              placeholder={copy.placeholder}
              type="email"
              required
            />
            <button type="submit">{copy.kicker}</button>
          </form>
        )}
      </section>
    </div>
  );
}

function ProcessIcon({ name }: { name: ProcessIconName }) {
  const paths: Record<ProcessIconName, ReactNode> = {
    auth: (
      <>
        <path d="M7 10V8a5 5 0 0 1 10 0v2" />
        <path d="M6 10h12v9H6z" />
        <path d="M12 14v2" />
      </>
    ),
    install: (
      <>
        <path d="M12 4v9" />
        <path d="m8.5 9.5 3.5 3.5 3.5-3.5" />
        <path d="M5 15v4h14v-4" />
      </>
    ),
    savings: (
      <>
        <path d="M5 7h14" />
        <path d="M7 11h10" />
        <path d="M9 15h6" />
        <path d="m16 5 3-3" />
      </>
    ),
  };

  return (
    <svg aria-hidden="true" className={styles.stepIcon} fill="none" viewBox="0 0 24 24">
      <g stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.9">
        {paths[name]}
      </g>
    </svg>
  );
}

function Icon({ name }: { name: string }) {
  const paths: Record<string, ReactNode> = {
    token: <path d="M4 8h16M7 12h10M9 16h6" />,
    diff: <path d="M6 6h7M6 12h12M6 18h9M17 5v4M15 7h4" />,
    lock: <path d="M7 11V8a5 5 0 0 1 10 0v3M6 11h12v9H6z" />,
    spark: <path d="m12 3 1.7 5.1L19 10l-5.3 1.9L12 17l-1.7-5.1L5 10l5.3-1.9z" />,
  };

  return (
    <svg aria-hidden="true" className={styles.icon} fill="none" viewBox="0 0 24 24">
      <g stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.8">
        {paths[name]}
      </g>
    </svg>
  );
}
