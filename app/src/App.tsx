import { linkIntent, useRoute } from "./router";
import { Invitation } from "./screens/Invitation";
import { Account } from "./screens/Account";
import { Assets, Passport } from "./screens/Vision";
import { SessionProvider, useSession } from "./session";
import { Page, Shell } from "./ui/Shell";
import { Welcome } from "./screens/Welcome";
import { Logo } from "./ui/kit";
import { Portfolio } from "./screens/Portfolio";
import { Vault } from "./screens/Vault";
import { NewBasket } from "./screens/NewBasket";

function Screens() {
  const route = useRoute();
  const { identity } = useSession();
  const intent = linkIntent(location.search);
  if (intent) return <Invitation intent={intent} />;
  if (!identity) return <Welcome />;
  const body = (() => {
    switch (route.name) {
      case "home": return <Portfolio />;
      case "new": return <NewBasket />;
      case "vault": return <Vault id={route.params.id} />;
      case "assets": return <Assets />;
      case "passport": return <Passport />;
      case "account": return <Account />;
      default: return <Page title="Not found" sub="This page does not exist." />;
    }
  })();
  return <Shell route={route.name}>{body}</Shell>;
}

export function App() {
  return (
    <SessionProvider fallback={(err) => (
      <main className="c-welcome"><div className="c-sky" style={{ display: "grid", placeItems: "center" }}>
        <div style={{ textAlign: "center" }}><Logo /><p className="muted" style={{ marginTop: 12 }}>{err ?? "Loading…"}</p></div>
      </div></main>
    )}>
      <Screens />
    </SessionProvider>
  );
}
