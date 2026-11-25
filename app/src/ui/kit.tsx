/** Small UI kit: logo, icons, badges. Shared language with the landing page. */
import type { ReactNode } from "react";

export function Logo({ label = true }: { label?: boolean }) {
  return (
    <span className="c-logo">
      <svg viewBox="11.75 11.75 70.5 70.5" aria-hidden="true">
        <path d="M11.75 11.75H57.2812V20.5625H66.0938V27.9062H27.9062V52.875H19.0938V61.6875H11.75V11.75Z" />
        <path d="M61.6875 11.75H82.25V57.2812H73.4375V66.0938H66.0938V32.3125H70.5V16.1562H61.6875V11.75Z" />
        <path d="M82.25 61.6875V82.25H11.75V66.0938H23.5V57.2812H27.9062V66.0938H61.6875V70.5H77.8438V61.6875H82.25Z" />
      </svg>
      {label && "Constella"}
    </span>
  );
}

const PATHS: Record<string, ReactNode> = {
  home: <><path d="M4 11l8-7 8 7" /><path d="M6 10v9h12v-9" /></>,
  new: <><path d="M12 5v14M5 12h14" /></>,
  assets: <><circle cx="8" cy="8" r="3.5" /><circle cx="16" cy="8" r="3.5" /><circle cx="12" cy="16" r="3.5" /></>,
  passport: <><rect x="5" y="3.5" width="14" height="17" rx="2.5" /><circle cx="12" cy="10.5" r="3" /><path d="M9 16.5h6" /></>,
  account: <><circle cx="12" cy="8.5" r="3.5" /><path d="M5 20c1.2-3.6 4-5.5 7-5.5s5.8 1.9 7 5.5" /></>,
  face: <><path d="M4 8V6a2 2 0 0 1 2-2h2M16 4h2a2 2 0 0 1 2 2v2M20 16v2a2 2 0 0 1-2 2h-2M8 20H6a2 2 0 0 1-2-2v-2" /><path d="M9 10v1M15 10v1M12 10v3h-1M9.5 15.5c1.5 1.2 3.5 1.2 5 0" /></>,
  phone: <><rect x="7" y="3" width="10" height="18" rx="2.5" /><path d="M11 18h2" /></>,
  wallet: <><rect x="3.5" y="6" width="17" height="13" rx="2.5" /><path d="M3.5 10h17M16 14.5h1.5" /><path d="M6 6l9-2.5 1 2.5" /></>,
  key: <><circle cx="8" cy="14" r="4" /><path d="M11 11l8-8M16 6l2 2M14 8l2 2" /></>,
  arrow: <><path d="M5 12h14M13 6l6 6-6 6" /></>,
};

export function Icon({ name }: { name: keyof typeof PATHS | string }) {
  return <svg className="c-ic" viewBox="0 0 24 24" aria-hidden="true">{PATHS[name]}</svg>;
}

export type BadgeKind = "live" | "soon" | "gated" | "warn" | "net";
export function Badge({ kind, children }: { kind: BadgeKind; children: ReactNode }) {
  return <span className={`c-badge c-badge-${kind}`}>{kind !== "net" && <i />}{children}</span>;
}

export function short(a: string, n = 4) {
  return a.length > 2 * n + 1 ? `${a.slice(0, n)}…${a.slice(-n)}` : a;
}
