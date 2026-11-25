/** A small history router (no dependency): paths -> screens, plus the
 * invitation links the relayer flows send (?pair, ?guard, ?recover). */
import { useEffect, useState } from "react";

export type RouteName = "home" | "new" | "vault" | "assets" | "passport" | "account" | "notfound";
export interface Route { name: RouteName; params: Record<string, string> }

const CONTRACT = /^C[A-Z2-7]{55}$/;
const STATIC: Record<string, RouteName> = { "": "home", new: "new", assets: "assets", passport: "passport", account: "account" };

export function matchRoute(pathname: string): Route {
  const parts = pathname.split("/").filter(Boolean);
  if (parts.length <= 1 && (parts[0] ?? "") in STATIC) return { name: STATIC[parts[0] ?? ""], params: {} };
  if (parts.length === 2 && parts[0] === "vault" && CONTRACT.test(parts[1])) return { name: "vault", params: { id: parts[1] } };
  return { name: "notfound", params: {} };
}

export type LinkIntent = { kind: "pair" | "guard" | "recover"; code: string } | null;

export function linkIntent(search: string): LinkIntent {
  const q = new URLSearchParams(search);
  for (const kind of ["pair", "guard", "recover"] as const) {
    const code = q.get(kind);
    if (code !== null) return /^[A-Z0-9]{8}$/.test(code) ? { kind, code } : null;
  }
  return null;
}

export function navigate(to: string) {
  if (to === location.pathname + location.search) return;
  history.pushState(null, "", to);
  dispatchEvent(new PopStateEvent("popstate"));
  scrollTo(0, 0);
}

export function useRoute(): Route {
  const [r, setR] = useState(() => matchRoute(location.pathname));
  useEffect(() => {
    const on = () => setR(matchRoute(location.pathname));
    addEventListener("popstate", on);
    return () => removeEventListener("popstate", on);
  }, []);
  return r;
}
