import {
  BellRing,
  KeyRound,
  RefreshCw,
  Settings2,
  ShieldCheck,
  SunMoon,
} from "lucide-react";
import { useEffect, useState } from "react";

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
  formatDateTime,
} from "../components/Primitives.js";

const DESKTOP_SETTINGS_CONFLICT =
  "Desktop settings changed in another window; reload before saving.";

export function SettingsPage() {
  const commands = useCommands();
  const resource = useCommandResource(async () => {
    const revision = appearanceChoiceRevision();
    const [settings, proxy] = await Promise.all([
      commands.getSettings(),
      commands.getProxyStatus(),
    ]);
    return { settings, proxy, revision };
  }, "settings");
  const [preview, setPreview] = useState<ProxyPreview | "loading" | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [update, setUpdate] = useState<UpdateStatus | null>(null);
  const [appearanceUnconfirmed, setAppearanceUnconfirmed] = useState(false);
  const appearance = useAppearancePreference();
  const announce = useAnnounce();

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

  const beginProxy = async () => {
    setPreview("loading");
    setError(null);
    try {
      setPreview(await commands.previewProxy());
    } catch (reason) {
      setPreview(null);
      setError(reason instanceof Error ? reason.message : String(reason));
    }
  };
  const changeProxy = async (enabled: boolean) => {
    if (enabled && (preview === null || preview === "loading")) return;
    setBusy("Proxy capture");
    setError(null);
    try {
      const status = await commands.setProxyEnabled(
        enabled,
        typeof preview === "object" && preview !== null
          ? preview.consentToken
          : null,
      );
      setPreview(null);
      const next =
        status.proxyStatus === "active"
          ? "Proxy capture is active."
          : enabled
            ? status.detail
            : "Proxy capture stopped.";
      setMessage(next);
      announce(next);
      resource.reload();
    } catch (reason) {
      setPreview(null);
      setError(reason instanceof Error ? reason.message : String(reason));
      resource.reload();
    } finally {
      setBusy(null);
    }
  };
  const proxy = resource.data?.proxy;

  return (
    <div className="page">
      <PageHeader
        eyebrow="Patch semantics · no hidden resets"
        title="Settings"
        description="Each control changes only its named field. Credentials live in the operating-system keychain, never in the settings file."
      />
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
              <p className="eyebrow">Display</p>
              <h2 id="appearance-settings-heading">Appearance</h2>
            </div>
          </div>
          <SettingsRow
            title="Color theme"
            description="Follow your device, or keep Cutokyo light or dark. This choice affects only Cutokyo."
          >
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
              <p className="eyebrow">Local privacy</p>
              <h2 id="privacy-settings-heading">Capture and agent access</h2>
            </div>
          </div>
          <SettingsRow
            title="Read-only Cutokyo search MCP"
            description="Let configured agents query attributable history. It exposes no mutation tools and never injects context automatically."
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
                aria-label="Toggle read-only Cutokyo search MCP"
              />
            </label>
          </SettingsRow>
          <SettingsRow
            title="Proxy capture fallback"
            description="Optional provider-traffic attribution. Review content, provider routing, and local redaction before enabling. Native capture remains the default."
          >
            <StatusPill
              state={
                proxy?.proxyStatus === "active"
                  ? "active"
                  : proxy?.proxyStatus === "failed"
                    ? "unavailable"
                    : "inactive"
              }
            >
              {proxy?.proxyStatus === "active"
                ? "Active"
                : proxy?.proxyStatus === "failed"
                  ? "Unavailable"
                  : "Inactive"}
            </StatusPill>
            <Button
              size="small"
              disabled={busy !== null || preview !== null}
              onClick={() =>
                proxy?.proxyEnabled
                  ? void changeProxy(false)
                  : void beginProxy()
              }
            >
              {proxy?.proxyEnabled ? "Stop proxy" : "Review before enabling"}
            </Button>
          </SettingsRow>
          <Disclosure>
            {proxy?.detail ??
              "Proxy runtime status is unavailable; no active listener is claimed."}{" "}
            Provider requests are forwarded unchanged. Credentials and raw
            payloads are not retained in local proxy traces.
          </Disclosure>
        </section>

        <section
          className="settings-section"
          aria-labelledby="storage-settings-heading"
        >
          <div className="settings-section__heading">
            <Settings2 aria-hidden="true" />
            <div>
              <p className="eyebrow">Storage</p>
              <h2 id="storage-settings-heading">Retention default</h2>
            </div>
          </div>
          <SettingsRow
            title="Stored history"
            description="A setting does not delete history by itself. Use Data controls to preview and apply the exact session set."
          >
            <select
              aria-label="Default history retention"
              value={
                data.retention_days === null
                  ? "keep"
                  : String(data.retention_days)
              }
              disabled={busy !== null}
              onChange={(event) => {
                const value = event.currentTarget.value;
                void patch("Retention default", {
                  retention_days: value === "keep" ? null : Number(value),
                });
              }}
            >
              <option value="keep">Keep until deleted</option>
              <option value="30">30 days</option>
              <option value="90">90 days</option>
              <option value="365">1 year</option>
            </select>
          </SettingsRow>
          <Disclosure>
            The local database is plaintext with owner-only permissions in v0.x.
            Use full-disk encryption. Deleting rows is not physical secure
            erasure from WAL, SSDs, snapshots, or existing backups.
          </Disclosure>
        </section>

        <section
          className="settings-section"
          aria-labelledby="update-settings-heading"
        >
          <div className="settings-section__heading">
            <BellRing aria-hidden="true" />
            <div>
              <p className="eyebrow">Application</p>
              <h2 id="update-settings-heading">Updates and crash records</h2>
            </div>
          </div>
          <SettingsRow
            title="Updater choice"
            description="Signed updater manifests are separate from platform code signing and notarization."
          >
            <select
              aria-label="Updater behavior"
              value={data.updater_choice}
              disabled={busy !== null}
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
            title="Bounded crash records"
            description="Offer a sanitized local crash record on next launch. Records are never uploaded automatically."
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
            title="Check for signed update"
            description="A failed network check does not change the current installation."
          >
            <Button
              size="small"
              disabled={busy !== null}
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
                <span>
                  Current {update.currentVersion} · checked{" "}
                  {formatDateTime(update.checkedAt)}
                </span>
              </div>
            </div>
          )}
        </section>

        <section
          className="settings-section settings-section--compact"
          aria-labelledby="secrets-heading"
        >
          <div className="settings-section__heading">
            <KeyRound aria-hidden="true" />
            <div>
              <p className="eyebrow">Credentials</p>
              <h2 id="secrets-heading">OS keychain only</h2>
            </div>
          </div>
          <p>
            Provider keys are never displayed here, written to TOML, or included
            in diagnostic bundles.
          </p>
        </section>
      </div>
      {preview === null ? null : (
        <Modal
          title="Enable local proxy capture?"
          description="Review this optional provider-routing fallback before any listener starts."
          size="wide"
          onClose={() => setPreview(null)}
          footer={
            <>
              <Button disabled={busy !== null} onClick={() => setPreview(null)}>
                Not now
              </Button>
              <Button
                variant="primary"
                disabled={preview === "loading" || busy !== null}
                onClick={() => void changeProxy(true)}
              >
                {busy === "Proxy capture"
                  ? "Starting…"
                  : "I understand — enable proxy"}
              </Button>
            </>
          }
        >
          {preview === "loading" ? (
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
                    term: "Native precedence",
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
  readonly description: string;
  readonly children: React.ReactNode;
}) {
  return (
    <div className="settings-row">
      <div>
        <strong>{title}</strong>
        <p>{description}</p>
      </div>
      <div className="settings-row__control">{children}</div>
    </div>
  );
}
