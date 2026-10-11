import styles from "./workflow-flow.module.css";

import type { CSSProperties } from "react";

// eslint-disable-next-line local/no-complex-business-logic -- These calculations define decorative SVG geometry and animation timing.
const streams = Array.from({ length: 28 }, (_, index) => {
  const lane = index % 14;
  const upper = index < 14;
  const path = upper
    ? `M-120 ${180 + lane * 13} C180 ${180 + lane * 13} 280 ${410 + lane * 5} 660 ${410 + lane * 5} S1120 ${80 + lane * 9} 1720 ${80 + lane * 9}`
    : `M-120 ${650 + lane * 10} C250 ${650 + lane * 10} 330 ${730 - lane * 5} 760 ${730 - lane * 5} S1260 ${470 + lane * 12} 1720 ${470 + lane * 12}`;
  return { path, duration: 14 + (index % 5) * 2, delay: -index * 2.7 };
});

/** Continuous context streams stay on their own routes, sharing one moving field. */
export function WorkflowFlow() {
  return (
    <div aria-hidden="true" className={styles.scene}>
      <svg fill="none" preserveAspectRatio="none" viewBox="0 0 1600 850">
        {streams.map((stream, index) => (
          <g
            key={index}
            style={
              {
                "--stream-duration": `${stream.duration}s`,
                "--stream-delay": `${stream.delay}s`,
              } as CSSProperties
            }
          >
            <path className={styles.route} d={stream.path} />
            <path className={styles.current} d={stream.path} pathLength={1000} />
          </g>
        ))}
      </svg>
    </div>
  );
}
