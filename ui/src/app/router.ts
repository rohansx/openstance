import { useEffect, useState } from 'react';

// A tiny hash router: `#/discover`, `#/people/812`. The token handoff
// (`#token=...`) is consumed before the router reads the hash.

export type Route =
  | { name: 'discover' }
  | { name: 'people' }
  | { name: 'person'; id: string }
  | { name: 'views' }
  | { name: 'playbooks'; playbook?: string; params?: Record<string, string> }
  | { name: 'shortlist' }
  | { name: 'intros' }
  | { name: 'sources' }
  | { name: 'review' }
  | { name: 'integrations' }
  | { name: 'agents' }
  | { name: 'egress' }
  | { name: 'settings' }
  | { name: 'notfound'; path: string };

const SIMPLE = ['discover', 'people', 'views', 'playbooks', 'shortlist', 'intros', 'sources', 'review', 'integrations', 'agents', 'egress', 'settings'] as const;

export function parseHash(hash: string): Route {
  const [path = '', search = ''] = hash.replace(/^#\/?/, '').split('?');
  const [head = '', second] = path.split('/').map((s) => decodeURIComponent(s));
  if (head === 'people' && second) return { name: 'person', id: second };
  if (head === 'playbooks' && second) {
    const params = Object.fromEntries(new URLSearchParams(search));
    return { name: 'playbooks', playbook: second, params };
  }
  if ((SIMPLE as readonly string[]).includes(head)) return { name: head } as Route;
  if (head === '') return { name: 'discover' };
  return { name: 'notfound', path: path.replace(/^\/+/, '') };
}

export function href(route: Route): string {
  switch (route.name) {
    case 'person':
      return `#/people/${encodeURIComponent(route.id)}`;
    case 'notfound':
      return `#/${route.path}`;
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
  notfound: 'Not found',
};
