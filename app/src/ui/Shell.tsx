/** Signed-in frame: left rail on desktop, bottom tabs on phones. */
import type { ReactNode } from "react";
import { navigate, type RouteName } from "../router";
import { useSession } from "../session";
import { Badge, Icon, Logo, short } from "./kit";

const NAV: { to: string; name: RouteName; label: string; icon: string }[] = [
  { to: "/", name: "home", label: "Portfolio", icon: "home" },
  { to: "/new", name: "new", label: "New basket", icon: "new" },
  { to: "/assets", name: "assets", label: "Assets", icon: "assets" },
  { to: "/passport", name: "passport", label: "Collateral Passport", icon: "passport" },
  { to: "/account", name: "account", label: "Account", icon: "account" },
];

function Link({ to, current, children }: { to: string; current: boolean; children: ReactNode }) {
  return (
    <a href={to} aria-current={current ? "page" : undefined}
       onClick={(e) => { if (e.metaKey || e.ctrlKey) return; e.preventDefault(); navigate(to); }}>{children}</a>
  );
}

export function Shell({ route, children }: { route: RouteName; children: ReactNode }) {
  const { identity } = useSession();
  const address = identity?.kind === "passkey" ? identity.account.address : identity?.signer.address ?? "";
  const label = identity?.kind === "wallet" ? "Stellar wallet" : "Passkey account";
  const current = (n: RouteName) => n === route || (n === "home" && route === "vault");
  return (
    <div className="c-shell">
      <aside className="c-rail">
        <a href="/" onClick={(e) => { e.preventDefault(); navigate("/"); }}><Logo /></a>
        <nav className="c-nav" aria-label="Main">
          {NAV.map((n) => <Link key={n.to} to={n.to} current={current(n.name)}><Icon name={n.icon} />{n.label}</Link>)}
        </nav>
        <div className="c-rail-foot">
          <Badge kind="net">Stellar testnet</Badge>
          <Link to="/account" current={false}>
            <span className="c-who"><span className="dot">{address.slice(0, 1)}</span>
              <span className="meta"><b>{label}</b><span>{short(address, 6)}</span></span></span>
          </Link>
        </div>
      </aside>
      <main className="c-main">
        <div className="c-topbar"><Logo /><Badge kind="net">Testnet</Badge></div>
        {children}
      </main>
      <nav className="c-tabs" aria-label="Main">
        {NAV.map((n) => <Link key={n.to} to={n.to} current={current(n.name)}><Icon name={n.icon} />{n.label.split(" ")[0]}</Link>)}
      </nav>
    </div>
  );
}

export function Page({ title, sub, actions, children }: { title: string; sub?: ReactNode; actions?: ReactNode; children?: ReactNode }) {
  return (
    <div className="c-page">
      <header className="c-head"><div><h1>{title}</h1>{sub && <p>{sub}</p>}</div>{actions}</header>
      {children}
    </div>
  );
}
