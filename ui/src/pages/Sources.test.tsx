import { act, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { fx } from '../test/fixtures';
import { installFakeXhr } from '../test/fakeXhr';
import { renderApp } from '../test/render';

describe('Sources and import', () => {
  it('lists imported sources with their stats', async () => {
    renderApp('#/sources');
    const list = await screen.findByRole('region', { name: /your imported sources/i });
    expect(await within(list).findByText('work gmail')).toBeInTheDocument();
    expect(within(list).getByText(/mbox · imported 2026-10-01 · 18,030 interactions · 3,120 identities/)).toBeInTheDocument();
    expect(within(list).getByText(/enrichment results/i)).toBeInTheDocument();
  });

  it('names the LinkedIn export .zip as an accepted format', async () => {
    renderApp('#/sources');
    const panel = await screen.findByRole('region', { name: /import your contacts/i });
    expect(within(panel).getByText(/LinkedIn export \.zip or Connections\.csv/)).toBeInTheDocument();
    expect(within(panel).getByRole('option', { name: /LinkedIn export \(\.zip or Connections\.csv\)/ })).toBeInTheDocument();
  });

  it('uploads files to POST /import with progress, then shows the import stats', async () => {
    const XHR = installFakeXhr();
    renderApp('#/sources');
    const panel = await screen.findByRole('region', { name: /import your contacts/i });
    const a = new File(['First Name,Last Name\n'], 'Connections.csv', { type: 'text/csv' });
    const b = new File(['BEGIN:VCALENDAR\n'], 'work.ics', { type: 'text/calendar' });
    await userEvent.upload(within(panel).getByLabelText(/choose export files/i), [a, b]);
    await userEvent.type(within(panel).getByRole('textbox', { name: /label/i }), 'linkedin 2026-10');
    await userEvent.click(within(panel).getByRole('button', { name: /^import 2 files/i }));

    const xhr = XHR.last!;
    expect(xhr.method).toBe('POST');
    expect(xhr.url).toBe('/import');
    expect(xhr.headers.authorization).toBe('Bearer test-token');
    expect(xhr.body!.getAll('file').map((f) => (f as File).name)).toEqual(['Connections.csv', 'work.ics']);
    expect(xhr.body!.get('label')).toBe('linkedin 2026-10');
    expect(xhr.body!.get('kind')).toBeNull();

    act(() => xhr.progress(50, 100));
    expect(within(panel).getByRole('progressbar', { name: /upload/i })).toHaveAttribute('value', '50');

    act(() => xhr.respond(200, fx.importStats()));
    const result = await within(panel).findByRole('region', { name: /import result/i });
    expect(within(result).getByText('1,210')).toBeInTheDocument();
    expect(within(result).getByText('3,402')).toBeInTheDocument();
    expect(within(result).getByRole('link', { name: /37 flagged/i })).toHaveAttribute('href', '#/review');
    expect(within(result).getByText(/linkedin_csv · linkedin 2026-10 · 4,812 identities · 4,812 interactions/)).toBeInTheDocument();
  });

  it('sends the importer kind and the mbox bodies opt-in when chosen', async () => {
    const XHR = installFakeXhr();
    renderApp('#/sources');
    const panel = await screen.findByRole('region', { name: /import your contacts/i });
    await userEvent.upload(within(panel).getByLabelText(/choose export files/i), new File(['From x'], 'work.mbox'));
    await userEvent.selectOptions(within(panel).getByRole('combobox', { name: /format/i }), 'mbox');
    await userEvent.click(within(panel).getByRole('checkbox', { name: /message bodies/i }));
    await userEvent.click(within(panel).getByRole('button', { name: /^import 1 file/i }));
    expect(XHR.last!.body!.get('kind')).toBe('mbox');
    expect(XHR.last!.body!.get('bodies')).toBe('true');
  });

  it('shows an import error and a skipped duplicate', async () => {
    const XHR = installFakeXhr();
    renderApp('#/sources');
    const panel = await screen.findByRole('region', { name: /import your contacts/i });
    await userEvent.upload(within(panel).getByLabelText(/choose export files/i), new File(['x'], 'Connections.csv'));
    await userEvent.click(within(panel).getByRole('button', { name: /^import 1 file/i }));
    const dup = fx.importStats();
    dup.sources[0]!.skipped_duplicate_file = true;
    act(() => XHR.last!.respond(200, dup));
    expect(await within(panel).findByText(/already imported/i)).toBeInTheDocument();

    await userEvent.upload(within(panel).getByLabelText(/choose export files/i), new File(['x'], 'bad.csv'));
    await userEvent.click(within(panel).getByRole('button', { name: /^import 1 file/i }));
    act(() => XHR.last!.respond(400, { v: 1, error: { code: 'bad_request', message: 'unrecognized file format' } }));
    await waitFor(() => expect(within(panel).getByRole('alert')).toHaveTextContent('unrecognized file format'));
  });
});

describe('import file input accessibility (U6)', () => {
  it('the file input has its own accessible name and description', async () => {
    renderApp('#/sources');
    const panel = await screen.findByRole('region', { name: /import your contacts/i });
    const input = panel.querySelector('input[type="file"]')!;
    expect(input).toHaveAttribute('id');
    expect(panel.querySelector(`label[for="${input.id}"]`)).toHaveTextContent(/choose export files/i);
    expect(input).toHaveAccessibleName('Choose export files');
    expect(input).toHaveAccessibleDescription(/linkedin.*mbox.*ics.*vcf/i);
  });
});
