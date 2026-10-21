import { useState } from 'react';
import { useApp } from '../app/context';
import { Icon, Logo } from './Icon';

/** Shown when this tab has no API token yet. */
export function ConnectGate() {
  const { setToken, health } = useApp();
  const [value, setValue] = useState('');
  return (
    <div className="connect">
      <div className="connect-card">
        <span className="brand">
          <Logo />
          openstance
        </span>
        <h1>Connect this tab to your graph.</h1>
        <p>
          Run <code>openstance serve</code> and open the link it prints (<code>http://127.0.0.1:7411/#token=…</code>). The
          token stays in this tab’s session storage and is removed from the address bar.
        </p>
        <form
          className="connect-form"
          onSubmit={(e) => {
            e.preventDefault();
            if (value.trim()) setToken(value.trim());
          }}
        >
          <label className="field">
            API token
            <input
              type="password"
              autoComplete="off"
              spellCheck={false}
              value={value}
              onChange={(e) => setValue(e.target.value)}
              placeholder="from ~/.openstance/api.token"
            />
          </label>
          <button type="submit" className="btn primary">
            Connect <Icon name="arrow" />
          </button>
        </form>
        <p className="note">
          <Icon name="shield" /> {health ? `Server reachable · openstance v${health.version}.` : 'Checking for a local server…'}{' '}
          The API listens on loopback only and needs the bearer token for every request.
        </p>
      </div>
    </div>
  );
}
