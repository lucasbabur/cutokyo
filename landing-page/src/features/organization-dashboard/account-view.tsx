import Link from "next/link";

import { AdminShell } from "@/shared/ui/admin-shell";

import styles from "./account-view.module.css";

type AccountUser = {
  createdAt: string;
  email: string;
  emailVerified: boolean;
  firstName: string | null;
  id: string;
  lastName: string | null;
  lastSignInAt: string | null;
  name: string | null;
  profilePictureUrl?: string | null;
};

// eslint-disable-next-line local/no-complex-business-logic -- Selects provider-avatar and sync-link presentation from WorkOS user data.
export function OrganizationAccountView({
  organizationId,
  permissions,
  role,
  signOutAction,
  user,
}: {
  organizationId: string;
  permissions: string[];
  role: string;
  signOutAction: () => Promise<void>;
  user: AccountUser;
}) {
  const name =
    user.name ?? ([user.firstName, user.lastName].filter(Boolean).join(" ") || user.email);
  const initials =
    [user.firstName, user.lastName]
      .filter(Boolean)
      .map((part) => part?.at(0))
      .join("") ||
    user.email.at(0)?.toUpperCase() ||
    "U";

  return (
    <AdminShell active="account" organizationId={organizationId} user={user}>
      <section className={styles.workspace}>
        <header className={styles.topbar}>
          <div>
            <p>Identity &amp; session</p>
            <h1>Your account</h1>
          </div>
          <Link className={styles.backButton} href="/admin">
            Back to overview
          </Link>
        </header>

        <section className={styles.profileHero}>
          <div className={styles.profileIdentity}>
            <div className={styles.avatar}>
              {user.profilePictureUrl ? (
                // eslint-disable-next-line @next/next/no-img-element -- Auth provider image hosts are supplied by WorkOS at runtime.
                <img
                  alt={`${name} profile`}
                  referrerPolicy="no-referrer"
                  src={user.profilePictureUrl}
                />
              ) : (
                initials
              )}
            </div>
            <div>
              <span>Organization identity</span>
              <h2>{name}</h2>
              <p>{user.email}</p>
              {!user.profilePictureUrl ? (
                <a
                  className={styles.syncPhoto}
                  href="/sign-in?returnTo=%2Fadmin%2Faccount&amp;reauth=google"
                >
                  Sync Google profile photo
                </a>
              ) : null}
            </div>
          </div>
          <div className={styles.sessionStatus}>
            <i />
            <div>
              <strong>Authenticated</strong>
              <small>Protected organization session</small>
            </div>
          </div>
        </section>

        <section className={styles.accountGrid}>
          <article className={styles.detailCard}>
            <header>
              <span>Profile</span>
              <h2>Personal information</h2>
            </header>
            <dl>
              <div>
                <dt>Email</dt>
                <dd>{user.email}</dd>
              </div>
              <div>
                <dt>Email status</dt>
                <dd className={user.emailVerified ? styles.verified : undefined}>
                  {user.emailVerified ? "Verified" : "Unverified"}
                </dd>
              </div>
              <div>
                <dt>Member since</dt>
                <dd>{formatDate(user.createdAt)}</dd>
              </div>
              <div>
                <dt>Last sign-in</dt>
                <dd>{user.lastSignInAt ? formatDate(user.lastSignInAt) : "First session"}</dd>
              </div>
            </dl>
          </article>

          <article className={styles.detailCard}>
            <header>
              <span>Organization</span>
              <h2>Access context</h2>
            </header>
            <dl>
              <div>
                <dt>Role</dt>
                <dd>{displayName(role)}</dd>
              </div>
              <div>
                <dt>Organization</dt>
                <dd className={styles.mono}>{organizationId}</dd>
              </div>
              <div>
                <dt>User ID</dt>
                <dd className={styles.mono}>{user.id}</dd>
              </div>
            </dl>
          </article>
        </section>

        <section className={styles.permissionsCard}>
          <header>
            <div>
              <span>Effective authorization</span>
              <h2>Permissions</h2>
            </div>
            <em>{permissions.length} granted</em>
          </header>
          <div className={styles.permissionList}>
            {permissions.map((permission) => (
              <span key={permission}>{permission}</span>
            ))}
            {!permissions.length ? <p>No organization permissions in this session.</p> : null}
          </div>
        </section>

        <section className={styles.dangerZone}>
          <div>
            <span>Session control</span>
            <h2>Sign out of Cutokyo</h2>
            <p>This clears the encrypted organization session cookie on this browser.</p>
          </div>
          <form action={signOutAction}>
            <button type="submit">Sign out</button>
          </form>
        </section>
      </section>
    </AdminShell>
  );
}

function formatDate(value: string) {
  return new Intl.DateTimeFormat("en", {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(value));
}

function displayName(value: string) {
  return value.replaceAll(/[-_:]+/g, " ").replace(/\b\w/g, (character) => character.toUpperCase());
}
