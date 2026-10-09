import { BellRing, RefreshCw, ShieldCheck, SunMoon } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import {
  appearanceChoiceRevision,
  setAppearancePreference,
  syncAppearancePreference,
  useAppearancePreference,
  type Appearance,
} from "../appearance.js";
import { useCommands, useCommandResource } from "../commands/context.js";
import type {
  DesktopSettings,
  ProxyPreview,
  UpdateStatus,
} from "../contracts.js";
import type { SettingsPatch } from "../generated/settings.js";
import { useAnnounce } from "../components/Announcer.js";
import { routeHref } from "../router.js";
import {
  Button,
  Disclosure,
  DefinitionList,
  Modal,
  ErrorState,
  LoadingState,
  PageHeader,
  StatusPill,
  SuccessMessage,
} from "../components/Primitives.js";

const DESKTOP_SETTINGS_CONFLICT =
  "Desktop settings changed in another window; reload before saving.";

export function SettingsPage({
  statusRefresh = 0,
}: {
  readonly statusRefresh?: number;
}) {
  const commands = useCommands();
  const resource = useCommandResource(async () => {
    const revision = appearanceChoiceRevision();
    const [settings, proxy, capabilities] = await Promise.all([
      commands.getSettings(),
      commands.getProxyStatus(),
      commands.getCapabilities(),
    ]);
    return { settings, proxy, capabilities, revision };
  }, `settings:${statusRefresh}`);
  const [preview, setPreview] = useState<ProxyPreview | "loading" | null>(null);
  const [previewError, setPreviewError] = useState<Error | null>(null);
  const previewGeneration = useRef(0);
  const proxyMutation = useRef(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [update, setUpdate] = useState<UpdateStatus | null>(null);
  const [appearanceUnconfirmed, setAppearanceUnconfirmed] = useState(false);
  const appearance = useAppearancePreference();
  const announce = useAnnounce();

  useEffect(
    () => () => {
      previewGeneration.current += 1;
    },
    [],
  );

  useEffect(() => {
    if (
      resource.state === "ready" &&
      !appearanceUnconfirmed &&
      resource.data.revision === appearanceChoiceRevision()
    ) {
      syncAppearancePreference(resource.data.settings.appearance);
    }
  }, [appearanceUnconfirmed, resource.data, resource.state]);

  if (resource.state === "loading" && resource.data === null) {
    return <LoadingState label="Reading settings and their origins" />;
  }
  if (resource.state === "error" && resource.data === null) {
    return (
      <ErrorState
        title="Settings are unavailable"
        error={resource.error}
        onRetry={resource.reload}
      />
    );
  }
  const data = resource.data?.settings ?? null;
  if (data === null) return null;

  const patch = async (name: string, settingsPatch: SettingsPatch) => {
    setBusy(name);
    setError(null);
    try {
      await commands.patchSettings(settingsPatch);
      const next = `${name} updated. Omitted settings were preserved.`;
      setMessage(next);
      announce(next);
      resource.reload();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setBusy(null);
    }
  };
  const patchPreference = async (
    name: string,
    settingsPatch: Readonly<
      Partial<Pick<DesktopSettings, "updater_choice" | "crash_reports_enabled">>
    >,
  ) => {
    setBusy(name);
    setError(null);
    try {
      await commands.patchDesktopPreferences(settingsPatch);
      const next = `${name} updated.`;
      setMessage(next);
      announce(next);
      resource.reload();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setBusy(null);
    }
  };
  const readWinningAppearance = async (): Promise<Appearance> => {
    const latest = await commands.getSettings();
    setAppearancePreference(latest.appearance);
    setAppearanceUnconfirmed(false);
    resource.reload();
    return latest.appearance;
  };
  const retryAppearanceRead = async () => {
    setBusy("Appearance");
    try {
      const winner = await readWinningAppearance();
      const confirmation = `Saved appearance refreshed to ${winner}.`;
      setError(null);
      announce(confirmation);
    } catch (reason) {
      setError(
        `Saved appearance is still unavailable. ${reason instanceof Error ? reason.message : String(reason)}`,
      );
    } finally {
      setBusy(null);
    }
  };
  const chooseAppearance = async (next: Appearance) => {
    if (busy !== null || appearanceUnconfirmed || next === appearance) return;
    const previous = appearance;
    setBusy("Appearance");
    setMessage(null);
    setError(null);
    setAppearancePreference(next);
    try {
      const saved = await commands.patchDesktopPreferences({
        appearance: next,
      });
      syncAppearancePreference(saved.appearance);
      const confirmation = `Appearance set to ${saved.appearance}.`;
      setMessage(confirmation);
      announce(confirmation);
      resource.reload();
    } catch (reason) {
      const detail = reason instanceof Error ? reason.message : String(reason);
      if (detail === DESKTOP_SETTINGS_CONFLICT) {
        setAppearanceUnconfirmed(true);
        try {
          const winner = await readWinningAppearance();
          const notice = `Appearance changed in another window. Saved appearance is ${winner}; your ${next} choice was not saved.`;
          setError(notice);
          announce(notice);
        } catch (readReason) {
          const notice = `Appearance changed in another window, but saved appearance could not be read. ${next} is a temporary preview, not a saved choice. ${readReason instanceof Error ? readReason.message : String(readReason)}`;
          setError(notice);
          announce(notice);
        }
      } else {
        setAppearancePreference(previous);
        const failure = `Could not save appearance. ${detail}`;
        setError(failure);
        announce(failure);
      }
    } finally {
      setBusy(null);
    }
  };
  const checkUpdate = async () => {
    setBusy("Updater");
    try {
      setUpdate(await commands.checkForUpdates());
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setBusy(null);
    }
  };

  const closeProxy = () => {
    if (proxyMutation.current) return;
    previewGeneration.current += 1;
    setPreview(null);
    setPreviewError(null);
  };
  const beginProxy = async () => {
    if (proxyMutation.current || busy !== null) return;
    const generation = ++previewGeneration.current;
    setPreview("loading");
    setPreviewError(null);
    setError(null);
    try {
      const next = await commands.previewProxy();
      if (generation === previewGeneration.current) setPreview(next);
    } catch (reason) {
      if (generation === previewGeneration.current) {
        setPreviewError(
          reason instanceof Error ? reason : new Error(String(reason)),
        );
      }
    }
  };
  const changeProxy = async (enabled: boolean) => {
    if (proxyMutation.current || busy !== null) return;
    if (
      enabled &&
      (proxyUnavailable ||
        previewError !== null ||
        preview === null ||
        preview === "loading")
    )
      return;
    proxyMutation.current = true;
    setBusy("Proxy capture");
    setPreviewError(null);
    setError(null);
    try {
      const status = await commands.setProxyEnabled(
        enabled,
        typeof preview === "object" && preview !== null
          ? preview.consentToken
          : null,
      );
      previewGeneration.current += 1;
      setPreview(null);
      const next =
        status.proxyStatus === "active"
          ? "Proxy capture is active."
          : enabled
            ? status.detail
            : proxyUnavailable
              ? "Proxy capture request cleared; no listener was running."
              : "Proxy capture stopped.";
      setMessage(next);
      announce(next);
      resource.reload();
    } catch (reason) {
      const failure =
        reason instanceof Error ? reason : new Error(String(reason));
      if (enabled) setPreviewError(failure);
      else setError(failure.message);
      resource.reload();
    } finally {
      proxyMutation.current = false;
      setBusy(null);
    }
  };
  const proxy = resource.data?.proxy;
  const proxyUnavailable =
    proxy === undefined || proxy.proxyStatus === "unavailable";

  return (
    <div className="page">
      <PageHeader title="Settings" />
      {resource.state === "error" ? (
        <ErrorState
          title="Settings could not refresh"
          error={resource.error}
          onRetry={resource.reload}
        />
      ) : null}
      {error === null ? null : (
        <div className="form-error form-error--action" role="alert">
          <span>{error}</span>
          {appearanceUnconfirmed ? (
            <Button
              size="small"
              disabled={busy !== null}
              onClick={() => void retryAppearanceRead()}
            >
              Retry reading saved appearance
            </Button>
          ) : null}
        </div>
      )}
      {message === null ? null : <SuccessMessage>{message}</SuccessMessage>}

      <div className="settings-sections">
        <section
          className="settings-section"
          aria-labelledby="appearance-settings-heading"
        >
          <div className="settings-section__heading">
            <SunMoon aria-hidden="true" />
            <div>
              <h2 id="appearance-settings-heading">Appearance</h2>
            </div>
          </div>
          <SettingsRow title="Theme">
            <div
              className="appearance-control"
              role="group"
              aria-label="Appearance"
            >
              {(["light", "dark", "system"] as const).map((option) => (
                <button
                  key={option}
                  type="button"
                  className="appearance-control__option"
                  aria-pressed={appearance === option}
                  aria-disabled={busy !== null || appearanceUnconfirmed}
                  onClick={() => void chooseAppearance(option)}
                >
                  {option === "system"
                    ? "System"
                    : option === "light"
                      ? "Light"
                      : "Dark"}
                </button>
              ))}
            </div>
          </SettingsRow>
        </section>
        <section
          className="settings-section"
          aria-labelledby="privacy-settings-heading"
        >
          <div className="settings-section__heading">
            <ShieldCheck aria-hidden="true" />
            <div>
              <h2 id="privacy-settings-heading">Capture</h2>
            </div>
          </div>
          <SettingsRow title="Agents">
            <a
              className="button button--regular button--secondary"
              href={routeHref("/onboarding")}
              aria-label="Manage capture"
            >
              Manage
            </a>
          </SettingsRow>
          <SettingsRow
            title="Agent search"
            description="Let your agents search past sessions (read-only)."
          >
            <label className="switch-control">
              <span>{data.search_mcp_enabled ? "Enabled" : "Disabled"}</span>
              <input
                type="checkbox"
                role="switch"
                checked={data.search_mcp_enabled}
                disabled={busy !== null}
                onChange={() =>
                  void patch("Search MCP", {
                    search_mcp_enabled: !data.search_mcp_enabled,
                  })
                }
                aria-label="Agent search"
              />
            </label>
          </SettingsRow>
          {proxyUnavailable &&
          !proxy?.proxyEnabled &&
          proxy?.proxyStatus !== "active" ? null : (
            <>
              <SettingsRow
                title="Proxy capture fallback"
                description="Route requests through a local proxy."
              >
                <StatusPill
                  state={
                    proxy?.proxyStatus === "active"
                      ? "active"
                      : proxyUnavailable
                        ? "unavailable"
                        : "inactive"
                  }
                >
                  {proxy?.proxyStatus === "active"
                    ? "Active"
                    : proxyUnavailable
                      ? "Unavailable"
                      : "Inactive"}
                </StatusPill>
                <Button
                  size="small"
                  disabled={
                    busy !== null ||
                    preview !== null ||
                    (!proxy?.proxyEnabled && proxyUnavailable)
                  }
                  onClick={() =>
                    proxy?.proxyEnabled
                      ? void changeProxy(false)
                      : void beginProxy()
                  }
                >
                  {proxy?.proxyEnabled
                    ? proxyUnavailable
                      ? "Clear proxy request"
                      : "Stop proxy"
                    : "Enable proxy"}
                </Button>
                {proxyUnavailable ? (
                  <Button
                    size="small"
                    variant="quiet"
                    disabled={busy !== null || preview !== null}
                    onClick={() => void beginProxy()}
                  >
                    Review data handling
                  </Button>
                ) : null}
              </SettingsRow>
            </>
          )}
        </section>

        <section
          className="settings-section"
          aria-labelledby="update-settings-heading"
        >
          <div className="settings-section__heading">
            <BellRing aria-hidden="true" />
            <div>
              <h2 id="update-settings-heading">Application</h2>
            </div>
          </div>
          <SettingsRow title="Updates">
            <select
              aria-label="Updater behavior"
              value={data.updater_choice}
              disabled={
                busy !== null || !resource.data?.capabilities.updates.available
              }
              onChange={(event) =>
                void patchPreference("Updater choice", {
                  updater_choice: event.currentTarget
                    .value as DesktopSettings["updater_choice"],
                })
              }
            >
              <option value="automatic">
                Download signed updates automatically
              </option>
              <option value="notify">Notify before downloading</option>
              <option value="manual">Check manually</option>
            </select>
          </SettingsRow>
          <SettingsRow
            title="Crash reports"
            description="Saved on this device only."
          >
            <label className="switch-control">
              <span>{data.crash_reports_enabled ? "Enabled" : "Disabled"}</span>
              <input
                type="checkbox"
                role="switch"
                checked={data.crash_reports_enabled}
                disabled={busy !== null}
                onChange={() =>
                  void patchPreference("Crash records", {
                    crash_reports_enabled: !data.crash_reports_enabled,
                  })
                }
                aria-label="Toggle bounded local crash records"
              />
            </label>
          </SettingsRow>
          <SettingsRow
            title="Check for updates"
            description={
              resource.data?.capabilities.updates.available === false
                ? "Install updates manually for now."
                : undefined
            }
          >
            <Button
              size="small"
              disabled={
                busy !== null || !resource.data?.capabilities.updates.available
              }
              onClick={() => void checkUpdate()}
            >
              <RefreshCw aria-hidden="true" />{" "}
              {busy === "Updater" ? "Checking…" : "Check now"}
            </Button>
          </SettingsRow>

          {update === null ? null : (
            <div className="update-result" role="status">
              <StatusPill
                state={
                  update.state === "current"
                    ? "healthy"
                    : update.state === "available"
                      ? "enabled"
                      : "unknown"
                }
              >
                {update.state}
              </StatusPill>
              <div>
                <strong>{update.detail}</strong>
                <span>Current {update.currentVersion}</span>
              </div>
            </div>
          )}
        </section>
      </div>
      {preview === null ? null : (
        <Modal
          title={
            proxyUnavailable
              ? "Proxy capture data handling"
              : "Enable local proxy capture?"
          }
          size="wide"
          closeDisabled={busy === "Proxy capture"}
          onClose={closeProxy}
          footer={
            <>
              <Button disabled={busy !== null} onClick={closeProxy}>
                Not now
              </Button>
              <Button
                variant="primary"
                disabled={
                  proxyUnavailable ||
                  previewError !== null ||
                  preview === "loading" ||
                  busy !== null
                }
                onClick={() => void changeProxy(true)}
              >
                {busy === "Proxy capture"
                  ? "Starting…"
                  : "I understand — enable proxy"}
              </Button>
            </>
          }
        >
          {previewError !== null ? (
            <ErrorState
              title={
                preview === "loading"
                  ? "Proxy preview is unavailable"
                  : "Proxy capture could not start"
              }
              error={previewError}
              onRetry={() => void beginProxy()}
            />
          ) : preview === "loading" ? (
            <LoadingState label="Preparing proxy boundary preview" />
          ) : (
            <div className="consent-grid">
              <DefinitionList
                rows={[
                  { term: "Listener", value: preview.bindAddress },
                  {
                    term: "Provider routing",
                    value:
                      "Your harness's configured provider and model; Cutokyo does not choose a different destination.",
                  },
                  {
                    term: "Capture order",
                    value: preview.fallbackBehavior,
                  },
                  { term: "Local redaction", value: preview.redactionBehavior },
                ]}
              />
              <section>
                <h3>Content passing through the proxy</h3>
                <ul>
                  {preview.inspectedContent.map((item) => (
                    <li key={item}>{item}</li>
                  ))}
                </ul>
              </section>
              <section>
                <h3>Never persisted by the proxy</h3>
                <ul>
                  {preview.neverPersisted.map((item) => (
                    <li key={item}>{item}</li>
                  ))}
                </ul>
              </section>
              <Disclosure>
                Content still leaves the machine for your configured model
                provider. Cancel sends nothing and leaves proxy capture off.
                Local diagnostic redaction does not block or change provider
                requests.
              </Disclosure>
            </div>
          )}
        </Modal>
      )}
    </div>
  );
}

function SettingsRow({
  title,
  description,
  children,
}: {
  readonly title: string;
  readonly description?: string | undefined;
  readonly children: React.ReactNode;
}) {
  return (
    <div className="settings-row">
      <div>
        <strong>{title}</strong>
        {description === undefined ? null : <p>{description}</p>}
      </div>
      <div className="settings-row__control">{children}</div>
    </div>
  );
}
