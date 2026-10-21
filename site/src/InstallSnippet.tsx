import { useState } from 'react';
import { Icon } from './Icon';
import { REPO } from './links';

export const INSTALL_LINES: [command: string, comment: string][] = [
  [`cargo install --git ${REPO} openstance`, 'installs openstance and the alias ost'],
  ['ost init --me you@example.com', 'creates ~/.openstance/graph.db'],
  ['ost import ~/Downloads/Takeout/Mail/*.mbox ~/Downloads/Connections.csv', 'mbox, LinkedIn CSV, .vcf, .ics, X .zip'],
  ['ost search "fintech founders who replied"', 'local, free, offline'],
  ['claude mcp add openstance -- openstance mcp', 'the same graph in Claude Code'],
];

type CopyState = 'idle' | 'copied' | 'failed';

export default function InstallSnippet() {
  const [state, setState] = useState<CopyState>('idle');
  const text = INSTALL_LINES.map(([cmd]) => cmd).join('\n');

  async function copy() {
    try {
      await navigator.clipboard.writeText(text);
      setState('copied');
    } catch {
      setState('failed');
    }
  }

  return (
    <div className="install-card">
      <div className="terminal-top">
        <span className="window-dots" aria-hidden="true">
          ● ● ●
        </span>
        <span>install · quickstart</span>
        <button type="button" className="copy-btn" onClick={copy}>
          <Icon name="copy" /> Copy
        </button>
      </div>
      <pre tabIndex={0} aria-label="Install and quickstart commands">
        {INSTALL_LINES.map(([cmd, comment]) => (
          <span className="install-line" key={cmd}>
            <span className="t-muted"># {comment}</span>
            {'\n'}
            <span className="t-muted">$ </span>
            {cmd}
            {'\n'}
          </span>
        ))}
        <span className="install-line">
          <span className="t-muted"># local web UI on 127.0.0.1:7411, arriving in this release</span>
          {'\n'}
          <span className="t-muted">$ </span>openstance serve
        </span>
      </pre>
      <p className="copy-status" role="status" aria-live="polite">
        {state === 'copied' ? 'Copied the five commands.' : state === 'failed' ? 'Copy failed. Select the text instead.' : ''}
      </p>
    </div>
  );
}
