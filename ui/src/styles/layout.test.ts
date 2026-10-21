// 375 px layout at the CSS level (P4-UI-06 acceptance: "works at 375 px").
//
// jsdom has no layout engine, so this test resolves the cascade by hand:
// it parses the stylesheets in the order main.tsx imports them, keeps the
// rules whose media queries match a 375 px wide viewport, and checks that
// nothing forces content wider than the viewport minus the 16 px gutters.
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import postcss, { type AtRule, type Rule } from 'postcss';
import { describe, expect, it } from 'vitest';

const VIEWPORT = 375;
const GUTTER = 16;
const CONTENT = VIEWPORT - 2 * GUTTER;
const FILES = ['prototype-base.css', 'prototype-workspace.css', 'prototype-integrations.css', 'app.css'];

type Decls = Map<string, Map<string, string>>; // selector -> prop -> value

function mediaMatches(params: string, width: number): boolean {
  const p = params.replace(/\s+/g, '');
  if (/prefers-reduced-motion/.test(p)) return false;
  const max = /max-width:(\d+)px/.exec(p);
  const min = /min-width:(\d+)px/.exec(p);
  if (max && width > Number(max[1])) return false;
  if (min && width < Number(min[1])) return false;
  return true;
}

function effective(width: number): Decls {
  const out: Decls = new Map();
  for (const f of FILES) {
    const root = postcss.parse(readFileSync(resolve(process.cwd(), 'src/styles', f), 'utf8'));
    root.walkRules((rule: Rule) => {
      const parent = rule.parent;
      if (parent && parent.type === 'atrule') {
        const at = parent as AtRule;
        if (at.name !== 'media' || !mediaMatches(at.params, width)) return;
      }
      for (const sel of rule.selectors) {
        const s = sel.trim();
        const props = out.get(s) ?? new Map<string, string>();
        rule.walkDecls((d) => {
          props.set(d.prop, d.value);
        });
        out.set(s, props);
      }
    });
  }
  return out;
}

/** Minimum width a grid-template-columns value forces, in px. */
function gridMin(value: string): number {
  let v = value.replace(/\s+/g, ' ').trim();
  // expand repeat(n, X)
  v = v.replace(/repeat\((\d+),\s*([^()]*(?:\([^()]*\))?[^()]*)\)/g, (_m, n: string, x: string) =>
    Array.from({ length: Number(n) }, () => x.trim()).join(' '),
  );
  const tracks = v.match(/minmax\([^)]*\)|\S+/g) ?? [];
  return tracks.reduce((sum, t) => {
    const mm = /^minmax\(\s*(\d+(?:\.\d+)?)px/.exec(t);
    if (mm) return sum + Number(mm[1]);
    const px = /^(\d+(?:\.\d+)?)px$/.exec(t);
    return px ? sum + Number(px[1]) : sum;
  }, 0);
}

const px = (v: string | undefined) => {
  const m = v && /^(\d+(?:\.\d+)?)px$/.exec(v.trim());
  return m ? Number(m[1]) : 0;
};

describe('375 px layout (CSS level)', () => {
  const at375 = effective(VIEWPORT);

  it('no rule forces a fixed width wider than the content area', () => {
    const offenders: string[] = [];
    for (const [sel, props] of at375) {
      for (const prop of ['width', 'min-width', 'flex-basis']) {
        const w = px(props.get(prop));
        if (w > CONTENT) offenders.push(`${sel} { ${prop}: ${props.get(prop)} }`);
      }
      const display = props.get('display') ?? 'grid';
      const cols = props.get('grid-template-columns');
      if (cols && display.includes('grid') && gridMin(cols) > CONTENT) {
        offenders.push(`${sel} { grid-template-columns: ${cols} } (min ${gridMin(cols)}px)`);
      }
    }
    expect(offenders).toEqual([]);
  });

  it('the shell stacks: sidebar on top, nav scrolls inside itself', () => {
    expect(at375.get('.shell')?.get('display')).toBe('block');
    expect(at375.get('.sidebar')?.get('position')).toBe('static');
    expect(at375.get('.sidebar')?.get('width')).toBe('100%');
    expect(at375.get('.nav')?.get('overflow')).toBe('auto');
    expect(at375.get('.main-shell')?.get('display')).toBe('block');
  });

  it('multi-column layouts collapse to one column', () => {
    const oneCol = (sel: string) => {
      const v = at375.get(sel)?.get('grid-template-columns') ?? '';
      expect(v.replace(/\s/g, ''), sel).toMatch(/^(1fr|minmax\(0,1fr\))$/);
    };
    for (const sel of ['.results-layout', '.section-grid', '.provider-grid', '.form-grid', '.person-page', '.ident-compare', '.board.board-5', '.playbook-grid', '.runner form']) {
      oneCol(sel);
    }
  });

  it('the main column keeps at least a 16 px side gutter', () => {
    const pad = (at375.get('.main')?.get('padding') ?? '').split(/\s+/).map((x) => px(x));
    const side = pad.length >= 2 ? pad[1]! : pad[0]!;
    expect(side).toBeGreaterThanOrEqual(GUTTER);
  });

  it('wide content scrolls or wraps inside its box', () => {
    expect(at375.get('.table-wrap')?.get('overflow')).toBe('auto');
    expect(at375.get('pre')?.get('white-space')).toBe('pre-wrap');
    expect(at375.get('.person-card')?.get('overflow-wrap')).toBe('anywhere');
  });

  it('every table in the app is inside a scrolling .table-wrap', () => {
    const sources = import.meta.glob(['../**/*.tsx', '!../**/*.test.tsx'], { query: '?raw', import: 'default', eager: true }) as Record<string, string>;
    for (const [file, text] of Object.entries(sources)) {
      const tables = (text.match(/<table[\s>]/g) ?? []).length;
      const wraps = (text.match(/table-wrap/g) ?? []).length;
      expect(wraps, file).toBeGreaterThanOrEqual(tables);
    }
  });
});
