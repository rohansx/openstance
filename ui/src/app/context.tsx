import { createContext, useCallback, useContext, useMemo, useRef, useState, type ReactNode } from 'react';
import { createClient, type Client } from '../api/client';
import { clearToken, readToken, saveToken } from '../api/token';
import type { ConfigJson, Health, Stats } from '../api/types';
import { useLoad } from '../lib/useLoad';

export interface AppCtx {
  api: Client;
  token: string | null;
  setToken: (t: string | null) => void;
  stats: Stats | undefined;
  health: Health | undefined;
  config: ConfigJson | undefined;
  refreshStats: () => void;
  toast: (msg: string) => void;
  toastText: string | null;
}

const Ctx = createContext<AppCtx | null>(null);

export function apiBase(): string {
  return (import.meta.env.VITE_API_BASE as string | undefined)?.trim() ?? '';
}

export function AppProvider({ initialToken, children }: { initialToken: string | null; children: ReactNode }) {
  const [token, setTokenState] = useState<string | null>(initialToken);

  const setToken = useCallback((t: string | null) => {
    if (t) saveToken(t);
    else clearToken();
    setTokenState(t);
  }, []);

  const api = useMemo(
    () =>
      createClient({
        base: apiBase(),
        getToken: readToken,
        onUnauthorized: () => setToken(null),
      }),
    [setToken],
  );

  const stats = useLoad(() => (token ? api.stats() : Promise.resolve(undefined)), [api, token]);
  const health = useLoad(() => api.health(), [api]);
  const config = useLoad(() => (token ? api.config() : Promise.resolve(undefined)), [api, token]);

  const [toastText, setToastText] = useState<string | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const toast = useCallback((msg: string) => {
    clearTimeout(timer.current);
    setToastText(msg);
    timer.current = setTimeout(() => setToastText(null), 4800);
  }, []);

  const value: AppCtx = {
    api,
    token,
    setToken,
    stats: stats.data,
    health: health.data,
    config: config.data,
    refreshStats: stats.reload,
    toast,
    toastText,
  };
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useApp(): AppCtx {
  const v = useContext(Ctx);
  if (!v) throw new Error('useApp outside AppProvider');
  return v;
}

/** Read a dotted key from the effective config (`enrich.backend`). */
export function configValue(cfg: ConfigJson | undefined, key: string): unknown {
  // `GET /config` wraps the effective config: { v, config, origins, ... }
  let cur: unknown = cfg && typeof cfg.config === 'object' ? cfg.config : cfg;
  for (const part of key.split('.')) {
    if (cur && typeof cur === 'object' && part in (cur as Record<string, unknown>)) {
      cur = (cur as Record<string, unknown>)[part];
    } else {
      return undefined;
    }
  }
  // `openstance config` annotates values with their origin: { value, origin }
  if (cur && typeof cur === 'object' && 'value' in (cur as Record<string, unknown>)) {
    return (cur as Record<string, unknown>).value;
  }
  return cur;
}
