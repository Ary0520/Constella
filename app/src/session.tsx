/** App session: testnet config (no secrets), chain client, and who is signed in. */
import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import { Chain, type Config } from "./core/chain";
import { actorFor, browserStore, type Actor, type Identity, PasskeyStore } from "./core/accounts";

interface Session {
  cfg: Config;
  chain: Chain;
  store: PasskeyStore;
  identity: Identity | null;
  /** a passkey saved but whose account was never created (interrupted setup) */
  pending: boolean;
  setIdentity(id: Identity | null): void;
  signOut(): void;
  actor(): Actor;
}

const Ctx = createContext<Session | null>(null);

export function useSession(): Session {
  const s = useContext(Ctx);
  if (!s) throw new Error("useSession outside SessionProvider");
  return s;
}

export function SessionProvider({ children, fallback }: { children: ReactNode; fallback: (err: string | null) => ReactNode }) {
  const [cfg, setCfg] = useState<Config | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const store = useMemo(() => new PasskeyStore(browserStore()), []);
  const [identity, setIdentityState] = useState<Identity | null>(() => {
    const a = store.load();
    return a?.address ? { kind: "passkey", account: a } : null;
  });
  const [pending, setPending] = useState(() => !!store.load() && !store.load()!.address);

  useEffect(() => {
    fetch("/config.json").then((r) => r.json()).then(setCfg).catch(() => setErr("Could not load the network settings. Reload the page."));
  }, []);
  const chain = useMemo(() => (cfg ? new Chain(cfg) : null), [cfg]);

  const setIdentity = useCallback((id: Identity | null) => {
    setIdentityState(id);
    setPending(!!store.load() && !store.load()!.address);
  }, [store]);
  const signOut = useCallback(() => { store.forget(); setIdentityState(null); setPending(false); }, [store]);

  if (!cfg || !chain) return <>{fallback(err)}</>;
  const value: Session = {
    cfg, chain, store, identity, pending, setIdentity, signOut,
    actor: () => {
      if (!identity) throw new Error("Sign in first.");
      return actorFor(identity, chain);
    },
  };
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}
