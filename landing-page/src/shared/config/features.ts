import "client-only";

import { env } from "@/shared/config/env";

/**
 * Whether token saving ships in this deployment.
 *
 * This is a build-level product switch, not the per-user desktop setting or the
 * per-organization policy default. When it is off the feature is absent, so its
 * screens are not rendered at all rather than rendered in an "off" state.
 */
export const tokenSavingEnabled = env.NEXT_PUBLIC_CUTOKYO_TOKEN_SAVING === "on";
