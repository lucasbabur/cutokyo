"use client";

import { useEffect, useRef } from "react";

import styles from "./hero-flow.module.css";
import { heroWaves } from "./hero-waves";

import type { VariationLocale } from "./i18n";
import type { CSSProperties } from "react";

const agents = [
  {
    id: "claude",
    name: "Claude Code",
    x: "50%",
    y: "var(--apex-y)",
    command: "Read the project context",
    lines: ["Reading src/app.tsx", "Found 12 shared context blocks", "Context ready for Codex ↗"],
  },
  {
    id: "codex",
    name: "Codex",
    x: "23%",
    y: "84%",
    command: "Continue the last session",
    lines: [
      "Loading session #482",
      "Reusing Claude's project memory",
      "Verified. Passing context ↗",
    ],
  },
  {
    id: "opencode",
    name: "OpenCode",
    x: "77%",
    y: "84%",
    command: "Resume with shared memory",
    lines: [
      "Syncing the working tree",
      "Session context received",
      "Same project. Ready to continue ↗",
    ],
  },
] as const;

const nativeTerminals = {
  claude: {
    screenshot: "/hero-agents/claude-terminal-current.png",
    viewport: "0 0 346 297",
    width: 346,
    height: 297,
    prompt: "❯",
  },
  codex: {
    screenshot: "/hero-agents/codex-terminal.png",
    viewport: "208 190 680 170",
    width: 1898,
    height: 1190,
    prompt: "›",
  },
  opencode: {
    screenshot: "/hero-agents/opencode-terminal-current.png",
    viewport: "136 160 928 482",
    width: 1200,
    height: 675,
    prompt: ">",
  },
} as const;

/** Horizontal waves run continuously across the hero, independently of the agents. */
export function HeroBackdrop() {
  return (
    <div aria-hidden="true" className={styles.backdrop}>
      <svg
        className={styles.ambientFlow}
        fill="none"
        preserveAspectRatio="none"
        viewBox="0 0 1800 1000"
      >
        <g className={styles.waveField}>
          {heroWaves.map((path) => (
            <path d={path} key={path} />
          ))}
        </g>
      </svg>
    </div>
  );
}

export function HeroFlow({ locale = "en" }: { locale?: VariationLocale }) {
  const sceneRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const scene = sceneRef.current;
    if (!scene || typeof IntersectionObserver === "undefined") return;
    const observer = new IntersectionObserver(([entry]) => {
      const paused = String(!entry?.isIntersecting);
      scene.dataset.paused = paused;
    });
    observer.observe(scene);
    return () => observer.disconnect();
  }, []);
  const description =
    locale === "pt-BR"
      ? "Claude, Codex e OpenCode trabalham em terminais, trocam contexto e se aproximam do centro até se juntarem na Cutokyo."
      : "Claude, Codex and OpenCode work in terminals, exchange context, and move together into the center to form Cutokyo.";
  return (
    <div aria-label={description} className={styles.scene} ref={sceneRef} role="img">
      <div aria-hidden="true" className={styles.stage}>
        <svg
          className={styles.connections}
          fill="none"
          preserveAspectRatio="none"
          viewBox="0 0 600 600"
        >
          <path d="M300 240C280 350 170 340 138 504" />
          <path d="M138 504C245 460 355 550 462 504" />
          <path d="M462 504C440 350 330 340 300 240" />
        </svg>
        {agents.map((agent) => (
          <div
            className={styles.agent}
            data-agent={agent.id}
            key={agent.id}
            style={
              {
                "--origin-y": agent.y,
                "--origin-x": agent.x,
                "--command-length": `${agent.command.length}ch`,
              } as CSSProperties
            }
          >
            <div className={styles.terminal}>
              <svg className={styles.nativeCapture} viewBox={nativeTerminals[agent.id].viewport}>
                <image
                  href={nativeTerminals[agent.id].screenshot}
                  width={nativeTerminals[agent.id].width}
                  height={nativeTerminals[agent.id].height}
                />
              </svg>
              {agent.id === "claude" ? (
                <svg className={styles.claudePromptCapture} viewBox="0 252 346 45">
                  <image
                    href={nativeTerminals.claude.screenshot}
                    width={nativeTerminals.claude.width}
                    height={nativeTerminals.claude.height}
                  />
                </svg>
              ) : null}
              <div className={styles.terminalBody}>
                <div className={styles.command}>
                  <span>{nativeTerminals[agent.id].prompt}</span>
                  <span className={styles.typedCommand}>{agent.command}</span>
                  <b className={styles.cursor}>▍</b>
                </div>
                <div className={styles.terminalLogs}>
                  {agent.lines.map((line, index) => (
                    <p className={styles.logLine} key={line}>
                      <span>{index === 2 ? "✓" : "·"}</span>
                      {line}
                    </p>
                  ))}
                </div>
              </div>
            </div>
            <div className={styles.agentMotion}>
              <div className={styles.companion}>
                <AgentSymbol agent={agent.id} />
              </div>
            </div>
          </div>
        ))}
        <div className={styles.energyCore} />
        <div className={styles.impactFlash} />
        <div className={styles.shockwave} />
        <div className={styles.shockwaveEcho} />
        <div className={styles.fragments}>
          {Array.from({ length: 12 }, (_, index) => (
            <i key={index} style={{ "--fragment-angle": `${index * 30}deg` } as CSSProperties} />
          ))}
        </div>
        <div className={styles.result}>
          <svg className={styles.cutokyoMark} viewBox="0 0 1024 1024">
            <path
              className={styles.markTop}
              d="M224 200h580v204l-128 52V336H348v176H224V200Z"
              fill="var(--text-primary)"
            />
            <path
              className={styles.markBottom}
              d="M224 512h124v176h328V568l128 52v204H224V512Z"
              fill="var(--text-primary)"
            />
            <g className={styles.markCore}>
              <circle cx="512" cy="512" fill="none" stroke="#c3f400" strokeWidth="48" r="88" />
            </g>
          </svg>
          <span className={styles.resultNote}>
            {locale === "pt-BR" ? "Uma memória compartilhada." : "One shared memory."}
          </span>
        </div>
      </div>
    </div>
  );
}

const artwork = {
  claude: {
    viewport: "6 7 61 42",
    height: 58,
    width: 75,
    source: "/hero-agents/claude-terminal-mascot.png",
  },
  codex: { viewport: "0 0 24 24", height: 24, width: 24, source: "/logos/codex.svg" },
  opencode: {
    viewport: "0 0 240 300",
    height: 300,
    width: 240,
    source: "/hero-agents/opencode-mark.svg",
  },
} as const;

function AgentSymbol({ agent }: { agent: keyof typeof artwork }) {
  const symbol = artwork[agent];
  return (
    <svg className={styles.symbol} viewBox={symbol.viewport}>
      {agent === "claude" ? (
        <defs>
          <filter id="claude-terminal-mascot-alpha" colorInterpolationFilters="sRGB">
            <feColorMatrix type="matrix" values="1 0 0 0 0  0 1 0 0 0  0 0 1 0 0  4 -4 0 0 0" />
          </filter>
        </defs>
      ) : null}
      <image
        filter={agent === "claude" ? "url(#claude-terminal-mascot-alpha)" : undefined}
        height={symbol.height}
        href={symbol.source}
        width={symbol.width}
      />
    </svg>
  );
}
