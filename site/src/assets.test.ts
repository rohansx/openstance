import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { describe, expect, it } from 'vitest';

const root = join(__dirname, '..');

function files(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    return statSync(path).isDirectory() ? files(path) : [path];
  });
}

const sources = [join(root, 'index.html'), ...files(join(root, 'src')), ...files(join(root, 'public'))].filter(
  (f) => /\.(html|css|tsx?|svg)$/.test(f) && !/\.test\.tsx?$/.test(f),
);

// Outbound links the reader may click are allowed; anything the browser would fetch on its own is not.
const fetchPatterns: [string, RegExp][] = [
  ['CSS url() or @import to another origin', /(url\(\s*['"]?|@import\s+['"]?)(https?:)?\/\//i],
  ['src= to another origin', /\bsrc\s*=\s*\{?\s*['"`](https?:)?\/\//i],
  ['<link href> to another origin', /<link\b[^>]*href\s*=\s*['"](https?:)?\/\//i],
  ['fetch/XHR/beacon/WebSocket', /\b(fetch|sendBeacon|XMLHttpRequest|WebSocket|EventSource)\s*\(/],
  ['font CDN', /fonts\.(googleapis|gstatic)\.com|cdnjs|jsdelivr|unpkg/i],
];

describe('static assets', () => {
  it('finds the sources to scan', () => {
    expect(sources.some((f) => f.endsWith('index.html'))).toBe(true);
    expect(sources.some((f) => f.endsWith('.css'))).toBe(true);
  });

  for (const [label, pattern] of fetchPatterns) {
    it(`make no external request: ${label}`, () => {
      const offenders = sources.filter((f) => pattern.test(readFileSync(f, 'utf8'))).map((f) => relative(root, f));
      expect(offenders).toEqual([]);
    });
  }

  it('only hard-codes https URLs that are the project repository or loopback examples', () => {
    const urls = sources.flatMap((f) => readFileSync(f, 'utf8').match(/https?:\/\/[^\s'"`<>)\\]+/g) ?? []);
    const unexpected = urls.filter(
      (u) =>
        !u.startsWith('https://github.com/rohansx/openstance') &&
        !u.startsWith('http://127.0.0.1:7411') &&
        !u.startsWith('http://www.w3.org/2000/svg'),
    );
    expect(unexpected).toEqual([]);
  });
});
