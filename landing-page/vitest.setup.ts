import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

import "@testing-library/jest-dom/vitest";

// Feature-rich suites opt in explicitly; dedicated tests cover the shipping-off default.
process.env.NEXT_PUBLIC_CUTOKYO_TOKEN_SAVING = "on";

afterEach(() => {
  cleanup();
});
