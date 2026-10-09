import Image from "next/image";
import Link from "next/link";

import styles from "./admin-shell.module.css";

import type { Route } from "next";
import type { ReactNode } from "react";

type AdminSection =
  | "account"
  | "activity"
  | "employees"
  | "enterprise"
  | "governance"
  | "overview"
  | "sessions";

type AdminIdentity = {
  email: string;
  firstName?: string | null;
  lastName?: string | null;
  profilePictureUrl?: string | null;
};

type AdminIconName =
  | "account"
  | "activity"
  | "employees"
  | "enterprise"
  | "governance"
  | "overview"
  | "sessions";

const NAVIGATION: { href: Route; icon: AdminIconName; key: AdminSection; label: string }[] = [
  { href: "/admin", icon: "overview", key: "overview", label: "Overview" },
  { href: "/admin/employees", icon: "employees", key: "employees", label: "Employees" },
  { href: "/admin/sessions", icon: "sessions", key: "sessions", label: "Sessions" },
  { href: "/admin/activity", icon: "activity", key: "activity", label: "Activity" },
  { href: "/admin/governance", icon: "governance", key: "governance", label: "Governance" },
  { href: "/admin/enterprise", icon: "enterprise", key: "enterprise", label: "Enterprise" },
  { href: "/admin/account", icon: "account", key: "account", label: "Account" },
];

export function AdminShell({
  active,
  children,
  organizationId,
  organizationName,
  user,
}: {
  active: AdminSection;
  children: ReactNode;
  organizationId: string;
  organizationName?: string | undefined;
  user: AdminIdentity;
}) {
  const name = [user.firstName, user.lastName].filter(Boolean).join(" ") || user.email;
  const initials =
    [user.firstName, user.lastName]
      .filter(Boolean)
      .map((part) => part?.at(0))
      .join("") ||
    user.email.at(0)?.toUpperCase() ||
    "U";

  return (
    <main className={styles.shell}>
      <aside className={styles.sidebar}>
        <Link className={styles.brand} href="/admin">
          <Image alt="" height={38} priority src="/brand/cutokyo-mark.svg" width={38} />
          <span>cutokyo</span>
        </Link>
        <p className={styles.sideLabel}>Organization console</p>
        <nav aria-label="Organization administration">
          {NAVIGATION.map((item) => (
            <Link
              aria-current={active === item.key ? "page" : undefined}
              href={item.href}
              key={item.key}
            >
              <AdminIcon name={item.icon} />
              <span>{item.label}</span>
              <em>→</em>
            </Link>
          ))}
        </nav>
        <Link className={styles.identity} href="/admin/account">
          <span className={styles.avatar}>
            {user.profilePictureUrl ? (
              // eslint-disable-next-line @next/next/no-img-element -- Auth provider image hosts are organization-defined at runtime.
              <img alt="" referrerPolicy="no-referrer" src={user.profilePictureUrl} />
            ) : (
              initials
            )}
          </span>
          <span className={styles.identityCopy}>
            <strong>{name}</strong>
            <small>{organizationName || organizationId}</small>
          </span>
          <em>→</em>
        </Link>
      </aside>
      {children}
    </main>
  );
}

function AdminIcon({ name }: { name: AdminIconName }) {
  const paths: Record<AdminIconName, ReactNode> = {
    account: (
      <>
        <circle cx="12" cy="8" r="3" />
        <path d="M5.5 19c.8-3.2 3-5 6.5-5s5.7 1.8 6.5 5" />
      </>
    ),
    activity: <path d="M3 12h4l2-5 4 10 2-5h6" />,
    employees: (
      <>
        <path d="M8.5 12a3 3 0 1 0 0-6 3 3 0 0 0 0 6Z" />
        <path d="M3 19c.5-3 2.3-4.5 5.5-4.5S13.5 16 14 19M15 7a2.5 2.5 0 0 1 0 5M16 14.5c2.8 0 4.4 1.5 4.8 4.5" />
      </>
    ),
    enterprise: (
      <>
        <path d="M4 20V6l8-3 8 3v14M8 8h1M8 12h1M8 16h1M15 8h1M15 12h1M15 16h1M10 20v-3h4v3" />
      </>
    ),
    governance: <path d="M12 3 5 6v5c0 4.6 2.7 7.8 7 10 4.3-2.2 7-5.4 7-10V6l-7-3Zm-3 9 2 2 4-5" />,
    overview: (
      <>
        <rect height="7" rx="1" width="7" x="3" y="3" />
        <rect height="7" rx="1" width="7" x="14" y="3" />
        <rect height="7" rx="1" width="7" x="3" y="14" />
        <rect height="7" rx="1" width="7" x="14" y="14" />
      </>
    ),
    sessions: (
      <>
        <path d="M4 5h16v11H9l-5 4V5Z" />
        <path d="M8 9h8M8 12h5" />
      </>
    ),
  };
  return (
    <svg aria-hidden="true" className={styles.navIcon} fill="none" viewBox="0 0 24 24">
      <g stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.5">
        {paths[name]}
      </g>
    </svg>
  );
}
