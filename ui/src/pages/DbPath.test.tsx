import { act, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { fx } from '../test/fixtures';
import { installFakeXhr } from '../test/fakeXhr';
import { renderApp } from '../test/render';

// U5: the database path comes from the API, never a hardcoded default.
// `GET /config` carries `db_path` (resolved) once the engine exposes it;
// `config.db.path` is the unexpanded setting (it reads "~/.openstance/graph.db"
// even when OPENSTANCE_HOME points elsewhere), so it is not used as the path.

function configWith(extra: Record<string, unknown>) {
  const c = fx.config();
  (c.config as Record<string, unknown>).db = { path: '~/.openstance/graph.db', busy_timeout_ms: 5000 };
  return { ...c, ...extra };
}

const HARDCODED = /~\/\.openstance\/graph\.db/;

async function onDevice() {
  return (await screen.findByRole('heading', { name: /on this device/i })).closest('section')!;
}

async function importOne() {
  const XHR = installFakeXhr();
  const panel = await screen.findByRole('region', { name: /import your contacts/i });
  await userEvent.upload(within(panel).getByLabelText(/choose export files/i), new File(['x'], 'Connections.csv'));
  await userEvent.click(within(panel).getByRole('button', { name: /^import 1 file/i }));
  act(() => XHR.last!.respond(200, fx.importStats()));
  return within(panel).findByRole('region', { name: /import result/i });
}

describe('database path (U5)', () => {
  it('Settings shows the resolved path from GET /config', async () => {
    renderApp('#/settings', { 'GET /config': configWith({ db_path: '/srv/home/openstance/graph.db' }) });
    const panel = await onDevice();
    expect(await within(panel).findByText('/srv/home/openstance/graph.db')).toBeInTheDocument();
    expect(within(panel).queryByText(HARDCODED)).toBeNull();
  });

  it('Settings falls back to neutral wording when the API has no resolved path', async () => {
    renderApp('#/settings', { 'GET /config': configWith({}) });
    const panel = await onDevice();
    await screen.findByText('enrich.backend');
    expect(within(panel).queryByText(HARDCODED)).toBeNull();
    expect(within(panel).getByText(/one local file, graph\.db, in your openstance home/i)).toBeInTheDocument();
  });

  it('the import result names the resolved path', async () => {
    renderApp('#/sources', { 'GET /config': configWith({ db_path: '/srv/home/openstance/graph.db' }) });
    const result = await importOne();
    expect(within(result).getByText(/everything stays in/i)).toHaveTextContent('/srv/home/openstance/graph.db');
  });

  it('the import result uses neutral wording without a resolved path', async () => {
    renderApp('#/sources', { 'GET /config': configWith({}) });
    const result = await importOne();
    expect(within(result).queryByText(HARDCODED)).toBeNull();
    expect(within(result).getByText(/everything stays in your local graph\.db on this device/i)).toBeInTheDocument();
  });
});
