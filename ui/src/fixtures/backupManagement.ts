import type {
  BackupInfo,
  BackupRestorePreview,
  CommandClient,
  SessionRecord,
} from "../contracts.js";
import { FIXTURE_NOW, type FixtureState } from "./scenarios.js";

/** In-memory fixture only; native recovery is verified independently in Rust. */
export function createBackupFixtureManagement(
  state: FixtureState,
  invalidatePreviews: () => void,
): Pick<
  CommandClient,
  "createBackup" | "listBackups" | "previewBackupRestore" | "restoreBackup"
> {
  const backups = new Map<
    string,
    { info: BackupInfo; sessions: SessionRecord[] }
  >();
  const previews = new Map<
    string,
    { preview: BackupRestorePreview; history: string }
  >();
  let sequence = 0;
  let busy = false;
  const scope =
    "Local session history, raw evidence, search projections, and summaries. Settings, harness configuration, and pending spool entries are excluded. Backups are private local files, not application-encrypted.";
  const pause = () => new Promise<void>((resolve) => setTimeout(resolve, 18));

  return {
    async createBackup(destination) {
      if (busy) throw new Error("Another history operation is running.");
      busy = true;
      try {
        await pause();
        const path =
          destination?.trim() ||
          `/isolated-cutokyo/backups/backup-${++sequence}`;
        if (backups.has(path))
          throw new Error(
            "Backup destination already exists; choose a new location.",
          );
        const sessions = structuredClone(state.sessions);
        const info: BackupInfo = {
          path,
          createdAt: FIXTURE_NOW,
          byteLength: new TextEncoder().encode(JSON.stringify(sessions)).length,
          sessionCount: sessions.length,
          scope,
        };
        backups.set(path, { info, sessions });
        return structuredClone(info);
      } finally {
        busy = false;
      }
    },
    async listBackups() {
      await pause();
      return [...backups.values()]
        .map(({ info }) => structuredClone(info))
        .reverse();
    },
    async previewBackupRestore(path) {
      await pause();
      const backup = backups.get(path);
      if (!backup) throw new Error("Backup directory is unavailable.");
      const preview: BackupRestorePreview = {
        previewToken: `restore-${++sequence}`,
        backup: structuredClone(backup.info),
        currentSessionCount: state.sessions.length,
      };
      previews.clear();
      previews.set(preview.previewToken, {
        preview,
        history: JSON.stringify(state.sessions),
      });
      return preview;
    },
    async restoreBackup(previewToken) {
      if (busy) throw new Error("Another history operation is running.");
      busy = true;
      try {
        await pause();
        const selection = previews.get(previewToken);
        if (!selection)
          throw new Error("Restore preview expired. Review the backup again.");
        if (selection.history !== JSON.stringify(state.sessions))
          throw new Error(
            "Current history changed after preview. Review the restore again.",
          );
        const backup = backups.get(selection.preview.backup.path);
        if (!backup)
          throw new Error("Backup is unavailable; history was not changed.");
        const recoveryPath = `/isolated-cutokyo/backups/recovery-${++sequence}`;
        // Keep a real fixture snapshot of the displaced history, not just a receipt.
        backups.set(recoveryPath, {
          info: {
            ...backup.info,
            path: recoveryPath,
            sessionCount: state.sessions.length,
          },
          sessions: structuredClone(state.sessions),
        });
        state.sessions = structuredClone(backup.sessions);
        previews.clear();
        invalidatePreviews();
        return {
          recoveryPath,
          restoredSessionCount: state.sessions.length,
          integrityResult: "ok",
        };
      } finally {
        busy = false;
      }
    },
  };
}
