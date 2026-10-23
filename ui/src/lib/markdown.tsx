import type { ReactNode } from 'react';

// A small, safe markdown renderer for playbook output (the `brief` markdown).
// It builds React elements only: no innerHTML, so any HTML in the source is
// shown as text. Supported: #-headings, - / * / 1. lists, > quotes, ``` code
// fences, paragraphs, **bold**, *em* / _em_, `code`, and [text](url) links for
// http(s) and mailto only.

const SAFE_URL = /^(https?:\/\/|mailto:)/i;

// One alternation, tried left to right at each position.
const INLINE =
  /(`[^`\n]+`)|(\*\*(?=\S)[^*\n]*?\S\*\*)|(\[[^\]\n]+\]\([^)\s]+\))|(\*(?=[^\s*])[^*\n]*?[^\s*]\*|\*[^\s*]\*)|((?<![\p{L}\p{N}_])_(?=[^\s_])[^_\n]*?[^\s_]_(?![\p{L}\p{N}_])|(?<![\p{L}\p{N}_])_[^\s_]_(?![\p{L}\p{N}_]))/u;

function inline(text: string, key = 'i'): ReactNode[] {
  const out: ReactNode[] = [];
  let rest = text;
  let n = 0;
  while (rest) {
    const m = INLINE.exec(rest);
    if (!m) {
      out.push(rest);
      break;
    }
    if (m.index > 0) out.push(rest.slice(0, m.index));
    const tok = m[0];
    const k = `${key}.${n++}`;
    if (m[1]) {
      out.push(<code key={k}>{tok.slice(1, -1)}</code>);
    } else if (m[2]) {
      out.push(<strong key={k}>{inline(tok.slice(2, -2), k)}</strong>);
    } else if (m[3]) {
      const split = tok.indexOf('](');
      const label = tok.slice(1, split);
      const url = tok.slice(split + 2, -1);
      out.push(
        SAFE_URL.test(url) ? (
          <a key={k} href={url} target="_blank" rel="noopener noreferrer">
            {inline(label, k)}
          </a>
        ) : (
          tok
        ),
      );
    } else {
      out.push(<em key={k}>{inline(tok.slice(1, -1), k)}</em>);
    }
    rest = rest.slice(m.index + tok.length);
  }
  return out;
}

type Block =
  | { kind: 'h'; level: number; text: string }
  | { kind: 'ul' | 'ol'; items: string[] }
  | { kind: 'quote'; lines: string[] }
  | { kind: 'code'; text: string }
  | { kind: 'p'; lines: string[] };

const UL = /^\s*[-*+]\s+(.*)$/;
const OL = /^\s*\d+[.)]\s+(.*)$/;
const H = /^(#{1,6})\s+(.*?)\s*#*\s*$/;
const Q = /^\s*>\s?(.*)$/;

function blocks(src: string): Block[] {
  const lines = src.replace(/\r\n?/g, '\n').split('\n');
  const out: Block[] = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i]!;
    if (!line.trim()) {
      i++;
      continue;
    }
    if (line.trimStart().startsWith('```')) {
      const body: string[] = [];
      i++;
      while (i < lines.length && !lines[i]!.trimStart().startsWith('```')) body.push(lines[i++]!);
      i++;
      out.push({ kind: 'code', text: body.join('\n') });
      continue;
    }
    const h = H.exec(line);
    if (h) {
      out.push({ kind: 'h', level: h[1]!.length, text: h[2]! });
      i++;
      continue;
    }
    const list = UL.test(line) ? UL : OL.test(line) ? OL : null;
    if (list) {
      const items: string[] = [];
      while (i < lines.length && list.test(lines[i]!)) items.push(list.exec(lines[i++]!)![1]!);
      out.push({ kind: list === UL ? 'ul' : 'ol', items });
      continue;
    }
    if (Q.test(line)) {
      const q: string[] = [];
      while (i < lines.length && Q.test(lines[i]!)) q.push(Q.exec(lines[i++]!)![1]!);
      out.push({ kind: 'quote', lines: q });
      continue;
    }
    const p: string[] = [];
    while (
      i < lines.length &&
      lines[i]!.trim() &&
      !UL.test(lines[i]!) &&
      !OL.test(lines[i]!) &&
      !H.test(lines[i]!) &&
      !Q.test(lines[i]!) &&
      !lines[i]!.trimStart().startsWith('```')
    ) {
      p.push(lines[i++]!.trim());
    }
    out.push({ kind: 'p', lines: p });
  }
  return out;
}

function withBreaks(lines: string[], key: string): ReactNode[] {
  return lines.flatMap((l, j) => (j === 0 ? inline(l, `${key}.${j}`) : [<br key={`${key}.br${j}`} />, ...inline(l, `${key}.${j}`)]));
}

/** Render markdown `source` as React elements. HTML in the source stays text. */
export function Markdown({ source, className = 'markdown' }: { source: string; className?: string }) {
  return (
    <div className={className}>
      {blocks(source).map((b, i) => {
        const k = `b${i}`;
        switch (b.kind) {
          case 'h': {
            // Inside a page section: start at h4 so the page outline holds.
            const Tag = (['h4', 'h5', 'h6'] as const)[Math.min(b.level, 3) - 1]!;
            return <Tag key={k}>{inline(b.text, k)}</Tag>;
          }
          case 'ul':
            return (
              <ul key={k}>
                {b.items.map((t, j) => (
                  <li key={j}>{inline(t, `${k}.${j}`)}</li>
                ))}
              </ul>
            );
          case 'ol':
            return (
              <ol key={k}>
                {b.items.map((t, j) => (
                  <li key={j}>{inline(t, `${k}.${j}`)}</li>
                ))}
              </ol>
            );
          case 'quote':
            return <blockquote key={k}>{withBreaks(b.lines, k)}</blockquote>;
          case 'code':
            return (
              <pre key={k}>
                <code>{b.text}</code>
              </pre>
            );
          default:
            return <p key={k}>{withBreaks(b.lines, k)}</p>;
        }
      })}
    </div>
  );
}
