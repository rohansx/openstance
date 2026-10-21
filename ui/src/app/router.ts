import { useEffect, useState } from 'react';

// A tiny hash router: `#/discover`, `#/people/812`. The token handoff
// (`#token=...`) is consumed before the router reads the hash.

export type Route =
  | { name: 'discover' }
  | { name: 'people' }
  | { name: 'person'; id: string }
  | { name: 'views' }
  | { name: 'playbooks'; playbook?: string }
  | { name: 'shortlist' }
  | { name: 'intros' }
  | { name: 'sources' }
  | { name: 'review' }
  | { name: 'integrations' }
  | { name: 'agents' }
  | { name: 'egress' }
  | { name: 'settings' };

const SIMPLE = ['discover', 'people', 'views', 'playbooks', 'shortlist', 'intros', 'sources', 'review', 'integrations', 'agents', 'egress', 'settings'] as const;

export function parseHash(hash: string): Route {
  const path = hash.replace(/^#\/?/, '').split('?')[0] ?? '';
  const [head = '', second] = path.split('/').map((s) => decodeURIComponent(s));
  if (head === 'people' && second) return { name: 'person', id: second };
  if (head === 'playbooks' && second) return { name: 'playbooks', playbook: second };
  if ((SIMPLE as readonly string[]).includes(head)) return { name: head } as Route;
  return { name: 'discover' };
}

export function href(route: Route): string {
  switch (route.name) {
    case 'person':
      return `#/people/${encodeURIComponent(route.id)}`;
    case 'playbooks':
      return route.playbook ? `#/playbooks/${encodeURIComponent(route.playbook)}` : '#/playbooks';
    default:
      return `#/${route.name}`;
  }
}

export function navigate(route: Route): void {
  window.location.hash = href(route).slice(1);
}

export function useRoute(): Route {
  const [route, setRoute] = useState<Route>(() => parseHash(window.location.hash));
  useEffect(() => {
    const on = () => setRoute(parseHash(window.location.hash));
    window.addEventListener('hashchange', on);
    return () => window.removeEventListener('hashchange', on);
  }, []);
  return route;
}

export const TITLES: Record<Route['name'], string> = {
  discover: 'Search people',
  people: 'People',
  person: 'Person',
  views: 'Saved views',
  playbooks: 'Playbooks',
  shortlist: 'Shortlist',
  intros: 'Introductions',
  sources: 'Sources',
  review: 'Review queue',
  integrations: 'Integrations',
  agents: 'Agent access',
  egress: 'Egress log',
  settings: 'Settings',
};
