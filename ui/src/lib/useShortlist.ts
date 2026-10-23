import { useCallback, useMemo } from 'react';
import { useApp } from '../app/context';
import { displayName } from './format';
import { useLoad } from './useLoad';

/** Shortlist membership with optimistic toggles (GET/POST/DELETE /shortlist). */
export function useShortlist() {
  const { api, toast } = useApp();
  const list = useLoad(() => api.shortlist(), [api]);
  const ids = useMemo(() => new Set((list.data ?? []).map((e) => e.person_id)), [list.data]);
  const { setData, reload } = list;
  const current = list.data;

  const toggle = useCallback(
    async (person: { id: number; display_name: string | null }) => {
      const had = ids.has(person.id);
      const before = current ?? [];
      setData(
        had
          ? before.filter((e) => e.person_id !== person.id)
          : [{ person_id: person.id, person: null, note: null, added_at: new Date().toISOString() }, ...before],
      );
      try {
        if (had) await api.removeFromShortlist(person.id);
        else await api.addToShortlist(person.id);
        reload();
        toast(had ? `Removed ${displayName(person)} from your shortlist.` : `Added ${displayName(person)} to your shortlist.`);
      } catch (e) {
        setData(before);
        toast(`Shortlist not updated: ${(e as Error).message}`);
      }
    },
    [api, ids, current, setData, reload, toast],
  );

  return { entries: list.data ?? [], ids, toggle, loading: list.loading, error: list.error, reload };
}
