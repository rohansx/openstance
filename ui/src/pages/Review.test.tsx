import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { renderApp } from '../test/render';

const EDITS = { 'POST /edits': { v: 1, applied: 1, people: [] } };
const ops = (api: ReturnType<typeof renderApp>['api']) => (api.callsTo('POST /edits').at(-1)!.body as { ops: unknown[] }).ops;

async function open() {
  const r = renderApp('#/review', EDITS);
  await screen.findByRole('heading', { name: 'Sam Ito' });
  return r;
}
const card = (name: string) => screen.getByRole('heading', { name }).closest('article')!;

describe('Review queue (P4-UI-04)', () => {
  it('shows flagged merges with confidence, rule, reason and both identity sets', async () => {
    await open();
    const sam = card('Sam Ito');
    expect(within(sam).getByText('85%')).toBeInTheDocument();
    expect(within(sam).getByText(/name_company/)).toBeInTheDocument();
    expect(within(sam).getByText('same name "sam ito"; company "acme"')).toBeInTheDocument();
    expect(within(sam).getByText(/in\/samito-example/)).toBeInTheDocument();
    expect(within(sam).getByText(/sam@acme\.example/)).toBeInTheDocument();
  });

  it('confirm writes a confirm op and reloads the queue', async () => {
    const { api } = await open();
    await userEvent.click(within(card('Sam Ito')).getByRole('button', { name: /^confirm merge/i }));
    await waitFor(() => expect(ops(api)).toEqual([{ op: 'confirm', person: 3 }]));
    await waitFor(() => expect(api.callsTo('GET /review').length).toBeGreaterThanOrEqual(2));
  });

  it('split sends the flagged identities', async () => {
    const { api } = await open();
    await userEvent.click(within(card('Sam Ito')).getByRole('button', { name: /^split/i }));
    await waitFor(() => expect(ops(api)).toEqual([{ op: 'split', person: 3, identities: [901] }]));
  });

  it('bulk confirms the selected merges in one batch, and by confidence threshold', async () => {
    const { api } = await open();
    await userEvent.click(within(card('Sam Ito')).getByRole('checkbox', { name: /select sam ito/i }));
    await userEvent.click(within(card('Jae Park')).getByRole('checkbox', { name: /select jae park/i }));
    await userEvent.click(screen.getByRole('button', { name: /confirm 2 selected/i }));
    await waitFor(() => expect(ops(api)).toEqual([{ op: 'confirm', person: 3 }, { op: 'confirm', person: 9 }]));

    await userEvent.click(screen.getByRole('button', { name: /confirm all at or above 85%/i }));
    await waitFor(() => expect(ops(api)).toEqual([{ op: 'confirm', person: 3 }]));
  });

  it('suggestions can be merged or kept apart', async () => {
    const { api } = await open();
    const sug = screen.getByRole('region', { name: /suggestions/i });
    expect(within(sug).getAllByText('Lena Ruiz')).toHaveLength(2);
    expect(within(sug).getByText('60%')).toBeInTheDocument();
    await userEvent.click(within(sug).getByRole('button', { name: /^merge/i }));
    await waitFor(() => expect(ops(api)).toEqual([{ op: 'merge', a: 40, b: 41 }]));
    await userEvent.click(within(sug).getByRole('button', { name: /keep apart/i }));
    await waitFor(() => expect(ops(api)).toEqual([{ op: 'reject', a: 40, b: 41 }]));
  });

  it('an empty queue says so', async () => {
    renderApp('#/review', { 'GET /review': { v: 1, flagged: [], suggestions: [], info: [] } });
    expect(await screen.findByText(/nothing to review/i)).toBeInTheDocument();
  });
});
