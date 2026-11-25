import { useRoute } from "./router";
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
  if (!identity) return <Welcome />;
  const body = (() => {
    switch (route.name) {
      case "home": return <Portfolio />;
      case "new": return <NewBasket />;
      case "vault": return <Vault id={route.params.id} />;
      case "assets": return <Page title="Assets" sub="The closed universe every basket is built from." />;
      case "passport": return <Page title="Collateral Passport" sub="Borrow against your basket without selling it." />;
      case "account": return <Page title="Account" sub="Devices, guardians and recovery." />;
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
