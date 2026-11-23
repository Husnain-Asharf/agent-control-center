import { useSyncExternalStore } from 'react';

export type RouteName =
  | 'overview'
  | 'sessions'
  | 'session'
  | 'approvals'
  | 'projects'
  | 'project'
  | 'activity'
  | 'analytics'
  | 'settings';

export interface Route {
  name: RouteName;
  param: string | null;
}

const NAMES: RouteName[] = ['overview', 'sessions', 'session', 'approvals', 'projects', 'project', 'activity', 'analytics', 'settings'];

export function parseHash(hash: string): Route {
  const [head, ...rest] = hash.replace(/^#\/?/, '').split('/');
  const name = NAMES.find((n) => n === head);
  if (!name) return { name: 'overview', param: null };
  const param = rest.join('/');
  return { name, param: param ? decodeURIComponent(param) : null };
}

export function go(hash: string): void {
  window.location.hash = hash.startsWith('#') ? hash : `#${hash}`;
}

let cached: { hash: string; route: Route } | null = null;

function getSnapshot(): Route {
  const hash = window.location.hash;
  if (!cached || cached.hash !== hash) cached = { hash, route: parseHash(hash) };
  return cached.route;
}

function subscribe(cb: () => void): () => void {
  window.addEventListener('hashchange', cb);
  return () => window.removeEventListener('hashchange', cb);
}

export function useRoute(): Route {
  return useSyncExternalStore(subscribe, getSnapshot);
}
