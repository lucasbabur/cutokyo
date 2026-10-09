"use client";

/* eslint-disable @next/next/no-img-element -- This landing page uses fixed local logo assets without adding extra wrappers. */

import { useCallback, useEffect, useRef, useState } from "react";

import { env } from "@/shared/config/env";

import { analyzeCutokyoContext } from "./api";
import { activeMarketingCopies } from "./i18n";
import styles from "./view.module.css";

import type { CutokyoAnalysisResult, CutokyoAnalysisStatus } from "./api";
import type { ProcessIconName, MarketingCopy, MarketingLocale } from "./i18n";
import type { CSSProperties, MouseEvent, ReactNode } from "react";
import type { BoxGeometry, Mesh, MeshBasicMaterial, Vector3 } from "three";

type CutokyoContextCategory = CutokyoAnalysisResult["analysis"]["categories"][number];

const proofLogos = [
  { name: "Stanford", src: "/logos/stanford-negative.png" },
  { name: "Forbes", src: "/logos/forbes-negative.png" },
  { name: "Wharton", src: "/logos/wharton-negative.png" },
  { name: "OpenAI", src: "/logos/openai-negative.png" },
] as const;

const logoCycles = [0, 1, 2] as const;

const NAV_SCROLL_OFFSET = 96;
const SCROLL_DURATION_MS = 900;

const DESKTOP_DOWNLOADS = [
  { asset: "Cutokyo-Windows-Setup.exe", key: "windows" },
  { asset: "Cutokyo-macOS-arm64.dmg", key: "macArm" },
  { asset: "Cutokyo-macOS-x64.dmg", key: "macIntel" },
  { asset: "Cutokyo-Linux.AppImage", key: "appImage" },
  { asset: "Cutokyo-Linux.deb", key: "deb" },
] as const;

export function MarketingSiteView({ locale = "en" }: { locale?: MarketingLocale }) {
  const copy: MarketingCopy = activeMarketingCopies[locale];
  const [isModalOpen, setIsModalOpen] = useState(false);
  const closeModal = useCallback(() => setIsModalOpen(false), []);
  const consoleState = useCutokyoConsole();
  useReveal();
  useReportMotion();

  const handleSectionLinkClick = (event: MouseEvent<HTMLAnchorElement>) => {
    const hash = event.currentTarget.hash;
    const target = hash ? document.getElementById(hash.slice(1)) : null;
    if (!target) return;

    event.preventDefault();
    const navOffset = hash === "#top" ? 0 : NAV_SCROLL_OFFSET;
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
          <img alt="" src="/brand/cutokyo-mark.svg" />
          <span>{copy.brand}</span>
        </a>
        <div>
          <a href="#proof" onClick={handleSectionLinkClick}>
            {copy.nav.trusted}
          </a>
          <a href="#how" onClick={handleSectionLinkClick}>
            {copy.nav.setup}
          </a>
          <a href="#inside" onClick={handleSectionLinkClick}>
            {copy.nav.inside}
          </a>
          <a href="#download" onClick={handleSectionLinkClick}>
            Download
          </a>
        </div>
        <button onClick={() => setIsModalOpen(true)} type="button">
          {copy.cta.requestDemo}
        </button>
      </nav>

      <section className={styles.hero} id="top">
        <ThreeHeroCubes />
        <div className={styles.heroCopy} data-reveal>
          <p className={styles.kicker}>{copy.hero.kicker}</p>
          <h1>{copy.hero.title}</h1>
          <p>{copy.hero.body}</p>
          <p className={styles.pricingNote}>{copy.hero.pricing}</p>
          <div className={styles.ctas}>
            <button onClick={() => setIsModalOpen(true)} type="button">
              {copy.cta.requestDemo}
            </button>
            <a href="#how" onClick={handleSectionLinkClick}>
              {copy.cta.seeWorkflow}
            </a>
            <a href="#download" onClick={handleSectionLinkClick}>
              Download desktop
            </a>
          </div>
        </div>
      </section>

      <section className={styles.logoBand} id="proof" aria-label={copy.nav.trusted}>
        <div className={styles.logoTrack}>
          {logoCycles.map((cycle) => (
            <div aria-hidden={cycle > 0} className={styles.logoGroup} key={cycle}>
              {proofLogos.map((logo) => (
                <span className={styles.logoTile} key={`${cycle}-${logo.name}`}>
                  <img alt={cycle === 0 ? `${logo.name} logo` : ""} src={logo.src} />
                </span>
              ))}
            </div>
          ))}
        </div>
      </section>

      <section className={styles.steps} id="how" aria-label={copy.nav.setup}>
        <h2 className="sr-only">{copy.nav.setup}</h2>
        {copy.steps.map((step, index) => (
          <article
            className={[styles.processCard, step.percent ? styles.savingsCard : ""].join(" ")}
            data-reveal
            key={step.title}
            style={{ "--reveal-delay": `${index * 180}ms` } as CSSProperties}
          >
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
          </article>
        ))}
      </section>

      <ContextControlPlane consoleState={consoleState} copy={copy} />

      <section className={styles.inside} id="inside">
        <div className={styles.sectionTitle} data-reveal>
          <p className={styles.kicker}>{copy.inside.kicker}</p>
          <h2>{copy.inside.title}</h2>
        </div>
        <div className={styles.iconGrid}>
          {copy.cards.map(([icon, title, body], index) => (
            <article
              data-reveal
              key={title}
              style={{ "--reveal-delay": `${(index % 2) * 120}ms` } as CSSProperties}
            >
              <Icon name={icon} />
              <h3>{title}</h3>
              <p>{body}</p>
            </article>
          ))}
        </div>
      </section>

      <DownloadSection copy={copy.download} />

      <footer className={styles.footer}>
        <div className={styles.footerBrand} data-reveal>
          <a href="#top" onClick={handleSectionLinkClick}>
            <img alt="" src="/brand/cutokyo-mark.svg" />
            <span>{copy.brand}</span>
          </a>
          <p>{copy.footer.description}</p>
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
          <a href="#download" onClick={handleSectionLinkClick}>
            Download
          </a>
          <button onClick={() => setIsModalOpen(true)} type="button">
            {copy.cta.requestDemo}
          </button>
        </div>
        <p className={styles.footerMeta}>{copy.footer.copyright}</p>
      </footer>

      {isModalOpen ? <DemoModal copy={copy.modal} onClose={closeModal} /> : null}
    </main>
  );
}

function DownloadSection({ copy }: { copy: MarketingCopy["download"] }) {
  const baseUrl =
    env.NEXT_PUBLIC_CUTOKYO_RELEASE_BASE_URL ??
    "https://github.com/lucasbabur/cutokyo/releases/latest/download";
  return (
    <section className={styles.download} id="download">
      <div className={styles.downloadIntro} data-reveal>
        <p className={styles.kicker}>{copy.kicker}</p>
        <h2>{copy.title}</h2>
        <p>{copy.body}</p>
      </div>
      <div className={styles.downloadGrid}>
        {DESKTOP_DOWNLOADS.map((download, index) => (
          <article
            data-reveal
            key={download.asset}
            style={{ "--reveal-delay": `${(index % 3) * 100}ms` } as CSSProperties}
          >
            <span>{String(index + 1).padStart(2, "0")}</span>
            <h3>{copy.platforms[download.key]}</h3>
            <code>{download.asset}</code>
            <div>
              <a href={`${baseUrl}/${download.asset}`}>Download</a>
              <a href={`${baseUrl}/${download.asset}.sha256`}>{copy.checksum}</a>
            </div>
          </article>
        ))}
      </div>
    </section>
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

function ThreeHeroCubes() {
  const containerRef = useRef<HTMLDivElement>(null);

  /* eslint-disable local/no-complex-business-logic -- Three.js render-loop math is visual animation, not business logic. */
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    if (!canUseWebGL()) return;
    let disposed = false;
    let frame = 0;
    let cleanup: (() => void) | undefined;

    const smooth = (edge0: number, edge1: number, value: number) => {
      const t = Math.min(1, Math.max(0, (value - edge0) / (edge1 - edge0)));
      return t * t * (3 - 2 * t);
    };

    void import("three").then((THREE) => {
      if (disposed || !container) return;

      const scene = new THREE.Scene();
      const camera = new THREE.PerspectiveCamera(42, 1, 0.1, 100);
      camera.position.set(0, 0, 13);

      let renderer: InstanceType<typeof THREE.WebGLRenderer>;
      try {
        renderer = new THREE.WebGLRenderer({ alpha: true, antialias: true });
      } catch {
        return;
      }
      renderer.setPixelRatio(Math.min(window.devicePixelRatio, 1.7));
      renderer.outputColorSpace = THREE.SRGBColorSpace;
      container.appendChild(renderer.domElement);

      const geometry = new THREE.BoxGeometry(1, 1, 1);
      const cubeGroup = new THREE.Group();
      scene.add(cubeGroup);
      const maxColumns = 12;
      const maxRows = 9;
      const layout = {
        columns: maxColumns,
        cubeSize: 0.34,
        exitX: 9,
        height: 10,
        marginX: 0.75,
        marginY: 0.75,
        rows: 15,
        trashX: -7,
        trashY: -5.5,
        usableHeight: 8.5,
        usableWidth: 14.5,
        width: 16,
      };
      const cubes: {
        active: boolean;
        base: Vector3;
        column: number;
        isRed: boolean;
        material: MeshBasicMaterial;
        mesh: Mesh<BoxGeometry, MeshBasicMaterial>;
        row: number;
        seed: number;
      }[] = [];

      for (let row = 0; row < maxRows; row += 1) {
        for (let column = 0; column < maxColumns; column += 1) {
          const isRed = (row + column) % 4 === 0 || (row % 5 === 1 && column % 6 === 2);
          const material = new THREE.MeshBasicMaterial({
            color: isRed ? 0xff315f : 0x32b7ff,
            transparent: true,
            opacity: 0,
          });
          const mesh = new THREE.Mesh(geometry, material);
          const base = new THREE.Vector3();
          mesh.frustumCulled = false;
          mesh.position.copy(base);
          mesh.rotation.set(row * 0.08, column * 0.05, (row + column) * 0.025);
          cubeGroup.add(mesh);
          cubes.push({
            active: true,
            base,
            column,
            isRed,
            material,
            mesh,
            row,
            seed: row * maxColumns + column,
          });
        }
      }

      const randomUnit = (seed: number) => {
        const value = Math.sin(seed * 12.9898 + 78.233) * 43758.5453;
        return value - Math.floor(value);
      };
      const starCount = 120;
      const starPositions = new Float32Array(starCount * 3);
      const starGeometry = new THREE.BufferGeometry();
      const starPositionAttribute = new THREE.Float32BufferAttribute(starPositions, 3);
      starGeometry.setAttribute("position", starPositionAttribute);
      const starMaterial = new THREE.PointsMaterial({
        color: 0xf5f4ea,
        opacity: 0.24,
        size: 0.05,
        transparent: true,
      });
      const stars = new THREE.Points(starGeometry, starMaterial);
      stars.position.z = -1.4;
      scene.add(stars);

      const refreshLayout = () => {
        const { width, height } = container.getBoundingClientRect();
        const visibleHeight =
          2 * Math.tan(THREE.MathUtils.degToRad(camera.fov / 2)) * camera.position.z;
        const visibleWidth = visibleHeight * camera.aspect;
        const aspect = visibleWidth / Math.max(visibleHeight, 1);
        layout.columns = aspect < 0.72 ? 5 : aspect < 1.1 ? 8 : maxColumns;
        layout.rows = aspect < 0.72 ? maxRows : aspect < 1.1 ? 7 : 6;
        layout.width = visibleWidth;
        layout.height = visibleHeight;
        layout.marginX = Math.min(visibleWidth * 0.22, (50 / Math.max(width, 1)) * visibleWidth);
        layout.marginY = Math.min(visibleHeight * 0.22, (50 / Math.max(height, 1)) * visibleHeight);
        layout.usableWidth = Math.max(visibleWidth - layout.marginX * 2, visibleWidth * 0.5);
        layout.usableHeight = Math.max(visibleHeight - layout.marginY * 2, visibleHeight * 0.5);
        layout.cubeSize = Math.max(
          0.09,
          Math.min(
            layout.usableWidth / Math.max(layout.columns + 1, 1),
            layout.usableHeight / Math.max(layout.rows + 1, 1),
          ) * 0.58,
        );
        layout.exitX = visibleWidth * 0.72 + layout.cubeSize * 4;
        layout.trashX = -visibleWidth / 2 + layout.marginX + layout.cubeSize * 1.6;
        layout.trashY = -visibleHeight / 2 + layout.marginY + layout.cubeSize * 1.6;

        cubes.forEach((cube) => {
          cube.active = cube.column < layout.columns && cube.row < layout.rows;
          cube.mesh.visible = cube.active;
          if (!cube.active) return;
          const columnProgress =
            layout.columns <= 1 ? 0.5 : cube.column / Math.max(layout.columns - 1, 1);
          const rowProgress = layout.rows <= 1 ? 0.5 : cube.row / Math.max(layout.rows - 1, 1);
          cube.base.set(
            (columnProgress - 0.5) * Math.max(layout.usableWidth - layout.cubeSize * 1.4, 0),
            (0.5 - rowProgress) * Math.max(layout.usableHeight - layout.cubeSize * 1.4, 0),
            0,
          );
        });

        for (let index = 0; index < starCount; index += 1) {
          const offset = index * 3;
          starPositions[offset] =
            -visibleWidth / 2 + layout.marginX + randomUnit(index) * layout.usableWidth;
          starPositions[offset + 1] =
            -visibleHeight / 2 + layout.marginY + randomUnit(index + 101) * layout.usableHeight;
          starPositions[offset + 2] = -randomUnit(index + 211) * 0.6;
        }
        starPositionAttribute.needsUpdate = true;
        starMaterial.size = Math.max(0.026, layout.cubeSize * 0.09);
      };

      const resize = () => {
        const { width, height } = container.getBoundingClientRect();
        renderer.setSize(Math.max(width, 1), Math.max(height, 1), false);
        camera.aspect = Math.max(width, 1) / Math.max(height, 1);
        camera.updateProjectionMatrix();
        refreshLayout();
      };

      const animate = () => {
        if (disposed) return;
        const time = performance.now() * 0.001;
        const cycle = (time % 10.8) / 10.8;
        const appear = smooth(0.02, 0.14, cycle) * (1 - smooth(0.9, 0.98, cycle));
        const redDiscard = smooth(0.2, 0.45, cycle);
        const merge = smooth(0.34, 0.68, cycle);
        const exit = smooth(0.68, 0.92, cycle);
        const blueFade = smooth(0.86, 0.98, cycle);
        starMaterial.opacity = 0.24 + smooth(0.72, 0.94, cycle) * 0.24;

        cubes.forEach(({ active, base, column, isRed, material, mesh, row, seed }) => {
          if (!active) return;
          if (isRed) {
            const trashScatterX = ((seed % 7) - 3) * layout.cubeSize * 0.38;
            const trashScatterY = (seed % 5) * layout.cubeSize * -0.18;
            mesh.position.set(
              base.x * (1 - redDiscard) + (layout.trashX + trashScatterX) * redDiscard,
              base.y * (1 - redDiscard) + (layout.trashY + trashScatterY) * redDiscard,
              -0.28 - redDiscard * 0.7,
            );
            mesh.scale.setScalar(layout.cubeSize * appear * (1 - redDiscard * 0.72));
            material.opacity = appear * (1 - redDiscard) * 0.88;
            mesh.rotation.x = time * 0.18 + row * 0.12;
            mesh.rotation.y = time * 0.16 + column * 0.08;
            mesh.rotation.z = time * 0.32 + seed * 0.02;
            return;
          }
          const clusterX = ((seed % 9) - 4) * layout.cubeSize * 0.16;
          const clusterY = ((Math.floor(seed / 9) % 7) - 3) * layout.cubeSize * 0.16;
          const mergedX = clusterX + exit * layout.exitX;
          const mergedY = clusterY + exit * Math.sin(seed * 0.27) * layout.cubeSize * 0.45;
          const targetX = base.x * (1 - merge) + mergedX * merge;
          const targetY = base.y * (1 - merge) + mergedY * merge;
          const targetZ = Math.sin(time * 1.2 + seed) * layout.cubeSize * 0.34 * (1 - merge);
          mesh.position.set(targetX, targetY, targetZ);
          mesh.scale.setScalar(layout.cubeSize * appear * (1 - merge * 0.18 + exit * 0.08));
          mesh.rotation.x = time * 0.09 + seed * 0.012;
          mesh.rotation.y = time * 0.11 + seed * 0.01;
          mesh.rotation.z = (row - column) * 0.018 + merge * 0.7;
          material.opacity = appear * (1 - blueFade) * (0.64 + merge * 0.26);
        });

        cubeGroup.rotation.x = Math.sin(time * 0.08) * 0.025;
        cubeGroup.rotation.y = Math.sin(time * 0.07) * 0.032;
        stars.rotation.z = time * 0.006;
        renderer.render(scene, camera);
        frame = requestAnimationFrame(animate);
      };

      resize();
      window.addEventListener("resize", resize);
      frame = requestAnimationFrame(animate);

      cleanup = () => {
        window.removeEventListener("resize", resize);
        cancelAnimationFrame(frame);
        geometry.dispose();
        starGeometry.dispose();
        starMaterial.dispose();
        cubes.forEach(({ material }) => material.dispose());
        renderer.dispose();
        container.replaceChildren();
      };
    });

    return () => {
      disposed = true;
      cleanup?.();
    };
  }, []);
  /* eslint-enable local/no-complex-business-logic */

  return (
    <div className={styles.heroCubes} aria-hidden="true">
      <div ref={containerRef} className={styles.heroCanvas} />
    </div>
  );
}

type CutokyoConsoleState = {
  contextPayload: string;
  error: string | null;
  result: CutokyoAnalysisResult | null;
  runAnalysis: () => Promise<void>;
  setContextPayload: (value: string) => void;
  status: CutokyoAnalysisStatus;
};

/* eslint-disable local/no-complex-business-logic -- This UI owns transient form state; request construction and analysis remain in infrastructure and FastAPI. */
function useCutokyoConsole(): CutokyoConsoleState {
  const [contextPayload, setContextPayload] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<CutokyoAnalysisResult | null>(null);
  const [status, setStatus] = useState<CutokyoAnalysisStatus>("idle");
  const controllerRef = useRef<AbortController | null>(null);

  useEffect(() => {
    return () => controllerRef.current?.abort();
  }, []);

  const runAnalysis = async () => {
    const payload = contextPayload.trim();
    if (!payload) return;
    controllerRef.current?.abort();
    const controller = new AbortController();
    controllerRef.current = controller;
    setStatus("analyzing");
    setError(null);
    setResult(null);
    try {
      const analysisResult = await analyzeCutokyoContext(payload, controller.signal);
      setResult(analysisResult);
      setStatus("live");
    } catch (analysisError: unknown) {
      if (analysisError instanceof DOMException && analysisError.name === "AbortError") return;
      setError(
        analysisError instanceof Error
          ? analysisError.message
          : "Live context analysis unavailable",
      );
      setStatus("error");
    }
  };

  return { contextPayload, error, result, runAnalysis, setContextPayload, status };
}

function ContextControlPlane({
  consoleState,
  copy,
}: {
  consoleState: CutokyoConsoleState;
  copy: MarketingCopy;
}) {
  const { contextPayload, error, result, runAnalysis, setContextPayload, status } = consoleState;

  return (
    <section className={styles.controlPlane} id="governance" aria-labelledby="control-plane-title">
      <div className={[styles.controlShell, styles.reportMotion].join(" ")} data-motion-delay="0">
        <div className={styles.controlChrome}>
          <div className={styles.windowControls} aria-hidden="true">
            <span />
            <span />
            <span />
          </div>
          <span>{copy.controlPlane.badge}</span>
          <em>context protected</em>
        </div>
        <div className={styles.controlGrid}>
          <div className={styles.controlColumn}>
            <div
              className={[styles.controlIntro, styles.reportMotion].join(" ")}
              data-motion-delay="80"
            >
              <p className={styles.kicker}>{copy.controlPlane.badge}</p>
              <h2 id="control-plane-title">{copy.controlPlane.title}</h2>
              <p>{copy.controlPlane.body}</p>
            </div>
            <form
              className={[styles.contextPreview, styles.reportMotion].join(" ")}
              data-motion-delay="180"
              onSubmit={(event) => {
                event.preventDefault();
                void runAnalysis();
              }}
            >
              <div className={styles.contextHeader}>
                <label htmlFor="context-analysis-payload">{copy.controlPlane.contextLabel}</label>
                <em>user supplied</em>
              </div>
              <textarea
                aria-label={copy.controlPlane.contextLabel}
                className={styles.contextInput}
                id="context-analysis-payload"
                onChange={(event) => setContextPayload(event.target.value)}
                placeholder={copy.controlPlane.inputPlaceholder}
                required
                value={contextPayload}
              />
              <div className={styles.analysisForm}>
                <button disabled={status === "analyzing" || !contextPayload.trim()} type="submit">
                  {status === "analyzing" ? "Analyzing…" : copy.controlPlane.runLabel}
                </button>
              </div>
            </form>
          </div>

          <div className={styles.analysisPanels}>
            {result ? <ContextAnalysisResult copy={copy} result={result} /> : null}
            {!result ? (
              <section
                className={styles.analysisPanel}
                role={status === "error" ? "alert" : "status"}
              >
                <span>Live analysis</span>
                <strong>
                  {status === "error"
                    ? "Unavailable"
                    : status === "analyzing"
                      ? "Analyzing…"
                      : copy.controlPlane.idleTitle}
                </strong>
                <p>
                  {error ??
                    (status === "analyzing"
                      ? "Waiting for the Cutokyo analysis API."
                      : copy.controlPlane.idleBody)}
                </p>
              </section>
            ) : null}
          </div>
        </div>
      </div>
    </section>
  );
}

function ContextAnalysisResult({
  copy,
  result,
}: {
  copy: MarketingCopy;
  result: CutokyoAnalysisResult;
}) {
  return (
    <>
      <section
        className={[styles.analysisPanel, styles.reportMotion].join(" ")}
        data-motion-delay="80"
      >
        <span>{copy.controlPlane.governanceTitle}</span>
        <strong>{result.governance.actor.id}</strong>
        <p>
          {result.governance.target.provider} / {result.governance.target.model}
        </p>
      </section>
      <section
        className={[styles.analysisPanel, styles.reportMotion].join(" ")}
        data-motion-delay="180"
      >
        <span>{copy.controlPlane.observabilityTitle}</span>
        <strong>{result.governance.logged ? copy.controlPlane.loggedLabel : "Pending"}</strong>
        <p>
          {copy.controlPlane.requestIdLabel}: <code>{result.governance.requestId}</code>
        </p>
        <p>
          {copy.controlPlane.logStreamLabel}: <code>back-end/logs/backend.jsonl</code>
        </p>
      </section>
      <section
        className={[styles.analysisPanel, styles.reportMotion].join(" ")}
        data-motion-delay="280"
      >
        <span>{copy.controlPlane.redactionTitle}</span>
        <strong>{result.security.redactionCount}</strong>
        <p>{securityCategoryText(result.security.categories)}</p>
      </section>
      <section
        className={[styles.categoryPanel, styles.reportMotion].join(" ")}
        data-motion-delay="380"
      >
        <span>{copy.controlPlane.categoriesTitle}</span>
        <strong>{formatTokenTotal(result.analysis.totalEstimatedTokens)} tokens</strong>
        <div className={styles.categoryList}>
          {result.analysis.categories.map((category, index) => (
            <CategoryMeter category={category} index={index} key={category.key} />
          ))}
        </div>
      </section>
    </>
  );
}
/* eslint-enable local/no-complex-business-logic */

function CategoryMeter({ category, index }: { category: CutokyoContextCategory; index: number }) {
  const meterStyle = { "--meter": `${Math.min(category.percent, 100)}%` } as CSSProperties;
  const delay = String(520 + index * 120);

  return (
    <div className={[styles.categoryRow, styles.reportMotion].join(" ")} data-motion-delay={delay}>
      <div>
        <strong>{category.label}</strong>
        <span>{category.purpose}</span>
      </div>
      <em>{category.percent.toFixed(1)}%</em>
      <i
        aria-hidden="true"
        className={styles.reportMeter}
        data-motion-delay={String(Number(delay) + 180)}
        style={meterStyle}
      />
    </div>
  );
}

function securityCategoryText(categories: Record<string, number>) {
  const entries = Object.entries(categories);
  if (!entries.length) return "No sensitive values found.";
  return entries.map(([label, count]) => `${label}: ${count}`).join(" · ");
}

function formatTokenTotal(value: number) {
  if (value >= 1000) return `${Math.round(value / 1000)}k`;
  return value.toLocaleString();
}

function canUseWebGL() {
  if (typeof document === "undefined") return false;
  if (navigator.userAgent.includes("jsdom")) return false;
  try {
    const canvas = document.createElement("canvas");
    return Boolean(canvas.getContext("webgl2") ?? canvas.getContext("webgl"));
  } catch {
    return false;
  }
}

function useReveal() {
  useEffect(() => {
    const els = Array.from(document.querySelectorAll<HTMLElement>("[data-reveal]"));
    if (!els.length) return;
    const deepLinkTarget = window.location.hash
      ? document.getElementById(window.location.hash.slice(1))
      : null;
    deepLinkTarget
      ?.querySelectorAll<HTMLElement>("[data-reveal]")
      .forEach((element) => element.setAttribute("data-visible", ""));
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
    const frame = requestAnimationFrame(() => {
      deepLinkTarget?.scrollIntoView({ block: "start" });
      const viewportHeight = window.innerHeight || document.documentElement.clientHeight || 1;
      els.forEach((element) => {
        const rect = element.getBoundingClientRect();
        if (rect.top < viewportHeight * 0.92 && rect.bottom > viewportHeight * 0.08) {
          element.setAttribute("data-visible", "");
          observer.unobserve(element);
        }
      });
    });
    return () => {
      cancelAnimationFrame(frame);
      observer.disconnect();
    };
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

function DemoModal({ copy, onClose }: { copy: MarketingCopy["modal"]; onClose: () => void }) {
  const dialogRef = useRef<HTMLElement>(null);
  const closeButtonRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    const previouslyFocused = document.activeElement;
    closeButtonRef.current?.focus();

    // eslint-disable-next-line local/no-complex-business-logic -- Keeps keyboard focus inside a presentational dialog.
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
        return;
      }

      if (event.key !== "Tab") return;
      const focusable = Array.from(
        dialogRef.current?.querySelectorAll<HTMLElement>("a[href], button:not([disabled])") ?? [],
      );
      const first = focusable[0];
      const last = focusable.at(-1);
      if (!first || !last) return;

      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    };

    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("keydown", handleKeyDown);
      if (previouslyFocused instanceof HTMLElement) previouslyFocused.focus();
    };
  }, [onClose]);

  return (
    <div className={styles.modalBackdrop} onMouseDown={onClose} role="presentation">
      <section
        aria-describedby="demo-note"
        aria-labelledby="demo-title"
        aria-modal="true"
        className={styles.modal}
        onMouseDown={(event) => event.stopPropagation()}
        ref={dialogRef}
        role="dialog"
      >
        <button
          aria-label={copy.closeLabel}
          className={styles.closeButton}
          onClick={onClose}
          ref={closeButtonRef}
          type="button"
        >
          ×
        </button>
        <p className={styles.kicker}>{copy.kicker}</p>
        <h2 id="demo-title">{copy.title}</h2>
        <p className={styles.modalNote} id="demo-note">
          {copy.note}
        </p>
        <a
          className={styles.modalContact}
          href="mailto:hello@cutokyo.com?subject=Cutokyo%20demo%20request"
        >
          {copy.contactLabel}
        </a>
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
    govern: (
      <>
        <path d="M12 3.5 5.5 6.2v5.1c0 4 2.7 7.6 6.5 9.2 3.8-1.6 6.5-5.2 6.5-9.2V6.2Z" />
        <path d="m9.3 12 1.9 1.9 3.5-3.6" />
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
