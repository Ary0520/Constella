import { useCallback, useEffect, useRef, useState } from "react";

/** Run an async read; re-run on deps or reload(). Ignores stale results. */
export function useAsync<T>(fn: () => Promise<T>, deps: unknown[]) {
  const [state, setState] = useState<{ data?: T; error?: string; loading: boolean }>({ loading: true });
  const seq = useRef(0);
  const run = useCallback(() => {
    const id = ++seq.current;
    setState((s) => ({ ...s, loading: true, error: undefined }));
    fn().then((data) => id === seq.current && setState({ data, loading: false }))
        .catch((e) => id === seq.current && setState({ error: String(e?.message ?? e), loading: false }));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);
  useEffect(run, [run]);
  return { ...state, reload: run };
}

/** A button action with busy / error / done states. */
export function useAction() {
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState<string | null>(null);
  const run = async (label: string, f: () => Promise<string | void>) => {
    setBusy(label); setError(null); setDone(null);
    try { const msg = await f(); if (msg) setDone(msg); } catch (e: any) { setError(String(e?.message ?? e).replace(/^Error:\s*/, "")); }
    finally { setBusy(null); }
  };
  return { busy, error, done, run, clear: () => { setError(null); setDone(null); } };
}
