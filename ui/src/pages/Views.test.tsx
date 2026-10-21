import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { renderApp } from '../test/render';

const card = (name: string) => screen.getByRole('heading', { name }).closest('article')!;

describe('Saved views (P4-UI-05)', () => {
  beforeEach(() => {
    vi.stubGlobal('URL', Object.assign(URL, { createObjectURL: vi.fn(() => 'blob:x'), revokeObjectURL: vi.fn() }));
  });

  it('lists own and imported views with rows, owner, expiry and exclusions', async () => {
    renderApp('#/views');
    await screen.findByRole('heading', { name: 'devtools-investors' });
    const imported = card('devtools-investors');
    expect(within(imported).getByText('IMPORTED · FROM ROHAN')).toBeInTheDocument();
    expect(within(imported).getByText('42')).toBeInTheDocument();
    expect(within(imported).getByText(/from rohan/)).toBeInTheDocument();
    expect(within(imported).getByText(/expires 2026-10-31/)).toBeInTheDocument();
    const own = card('fintech-founders');
    expect(within(own).getByText('12')).toBeInTheDocument();
    expect(within(own).getByText(/3 left out: do not share/)).toBeInTheDocument();
    expect(within(own).getByText(/never expires/)).toBeInTheDocument();
  });

  it('exports an own view as an encrypted .ost download', async () => {
    const { api } = renderApp('#/views', {
      'POST /views/fintech-founders/export': { raw: 'age-encryption.org/v1\n', contentType: 'application/octet-stream' },
    });
    await screen.findByRole('heading', { name: 'fintech-founders' });
    const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {});
    await userEvent.click(within(card('fintech-founders')).getByRole('button', { name: /export \.ost/i }));
    await waitFor(() => expect(api.callsTo('POST /views/fintech-founders/export')).toHaveLength(1));
    await waitFor(() => expect(click).toHaveBeenCalled());
    const a = click.mock.contexts[0] as HTMLAnchorElement;
    expect(a.download).toBe('fintech-founders.ost');
  });

  it('creates a view from a query text', async () => {
    const { api } = renderApp('#/views', {
      'POST /views': { v: 1, id: null, name: 'investors', direction: 'own', owner: { pubkey: 'age1me', label: null }, created_at: '2026-10-21T00:00:00Z', expires_at: null, rows: 5, query: { text: 'devtools investor' } },
    });
    await screen.findByRole('heading', { name: 'devtools-investors' });
    await userEvent.click(screen.getByRole('button', { name: /new view/i }));
    const dialog = screen.getByRole('dialog');
    await userEvent.type(within(dialog).getByRole('textbox', { name: /view name/i }), 'investors');
    await userEvent.type(within(dialog).getByRole('textbox', { name: /query/i }), 'devtools investor');
    await userEvent.click(within(dialog).getByRole('button', { name: /save view/i }));
    await waitFor(() =>
      expect(api.callsTo('POST /views')[0]?.body).toEqual({ name: 'investors', query: 'devtools investor', ttl: '30d', to: ['self'] }),
    );
    await waitFor(() => expect(api.callsTo('GET /views').length).toBeGreaterThanOrEqual(2));
  });

  it('searches inside an imported view', async () => {
    const { api } = renderApp('#/views', {
      'GET /views/devtools-investors/search': {
        v: 1,
        results: [
          { view: { id: '01JC50ZZZZZZZZ', name: 'devtools-investors', owner: { label: 'rohan' } },
            person: { display_name: 'Maya Chen', title: 'Partner', company: 'Foo Ventures', location: 'NYC', score_bucket: 'strong', last_contact_month: '2026-07', emails: ['maya@foo.example'], linkedin: 'in/mayachen-example' },
            relevance: 0.71 },
        ],
      },
    });
    await screen.findByRole('heading', { name: 'devtools-investors' });
    const imported = card('devtools-investors');
    await userEvent.type(within(imported).getByRole('searchbox', { name: /search devtools-investors/i }), 'partner{Enter}');
    expect(await within(imported).findByText('Maya Chen')).toBeInTheDocument();
    expect(within(imported).getByText(/strong/)).toBeInTheDocument();
    expect(api.callsTo('GET /views/devtools-investors/search')[0]!.url.searchParams.get('q')).toBe('partner');
  });

  it('imports a .ost file', async () => {
    const { api } = renderApp('#/views', {
      'POST /views/import': { v: 1, id: '01JD', name: 'from-ana', direction: 'imported', owner: { pubkey: null, label: 'ana' }, created_at: '2026-10-21T00:00:00Z', expires_at: null, rows: 9, query: {} },
    });
    await screen.findByRole('heading', { name: 'devtools-investors' });
    const file = new File(['age-encryption.org/v1'], 'from-ana.ost', { type: 'application/octet-stream' });
    await userEvent.upload(screen.getByLabelText(/import a \.ost file/i), file);
    await waitFor(() => expect(api.callsTo('POST /views/import')).toHaveLength(1));
    const body = api.callsTo('POST /views/import')[0]!.body as FormData;
    expect((body.get('file') as File).name).toBe('from-ana.ost');
  });
});
