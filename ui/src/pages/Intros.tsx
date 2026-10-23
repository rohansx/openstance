import { useState } from 'react';
import { INTRO_STAGES, type HowConnected, type Intro, type IntroStage } from '../api/types';
import { useApp } from '../app/context';
import { href } from '../app/router';
import { ErrorNote, Loading, PageHead } from '../components/bits';
import { Dialog } from '../components/Dialog';
import { Icon } from '../components/Icon';
import { displayName } from '../lib/format';
import { useLoad } from '../lib/useLoad';

export const STAGE_LABEL: Record<IntroStage, string> = {
  draft: 'Draft',
  asked: 'Asked',
  intro_made: 'Intro made',
  met: 'Met',
  closed: 'Closed',
};

export function Intros() {
  const { api } = useApp();
  const intros = useLoad(() => api.intros(), [api]);
  const [creating, setCreating] = useState(false);
  const all = intros.data ?? [];
  return (
    <>
      <PageHead
        eyebrow="FROM CONNECTION TO CONVERSATION"
        title="Make the introduction count."
        sub="Your local introduction tracker. You move the stages; openstance never sends a message."
        aside={
          <button type="button" className="btn primary" onClick={() => setCreating(true)}>
            <Icon name="plus" />
            New introduction
          </button>
        }
      />
      <ErrorNote error={intros.error} />
      {intros.loading && !intros.data && <Loading what="Loading introductions" />}
      <div className="board board-5">
        {INTRO_STAGES.map((stage) => {
          const rows = all.filter((i) => i.stage === stage);
          return (
            <section className="lane" key={stage} aria-label={`${STAGE_LABEL[stage]} (${rows.length})`}>
              <h2>
                {STAGE_LABEL[stage]}
                <span>{rows.length}</span>
              </h2>
              {rows.length ? (
                rows.map((i) => <IntroCard key={i.id} intro={i} onChanged={intros.reload} />)
              ) : (
                <p className="lane-empty">Room for a new conversation.</p>
              )}
            </section>
          );
        })}
      </div>
      {creating && (
        <NewIntroDialog
          onClose={() => setCreating(false)}
          onCreated={() => {
            setCreating(false);
            intros.reload();
          }}
        />
      )}
    </>
  );
}

function IntroCard({ intro, onChanged }: { intro: Intro; onChanged: () => void }) {
  const { api, toast, refreshStats } = useApp();
  const name = intro.person ? displayName(intro.person) : `Person #${intro.person_id}`;
  const act = async (fn: () => Promise<unknown>, done: string) => {
    try {
      await fn();
      toast(done);
      onChanged();
      refreshStats();
    } catch (e) {
      toast((e as Error).message);
    }
  };
  return (
    <article className="intro-card">
      <h3>
        <a href={href({ name: 'person', id: String(intro.person_id) })}>{name}</a>
      </h3>
      <p>{[intro.person?.title, intro.person?.company].filter(Boolean).join(' · ') || 'Contact'}</p>
      <p className="via">{intro.connector ? `via ${displayName(intro.connector)}` : 'Direct ask'}</p>
      {intro.note && <p>{intro.note}</p>}
      {intro.follow_up && (
        <div className="due">
          <Icon name="clock" /> Follow up {intro.follow_up}
        </div>
      )}
      <select
        aria-label={`Stage for ${name}`}
        value={intro.stage}
        onChange={(e) => void act(() => api.updateIntro(intro.id, { stage: e.target.value as IntroStage }), 'Stage updated.')}
      >
        {INTRO_STAGES.map((s) => (
          <option key={s} value={s}>
            {STAGE_LABEL[s]}
          </option>
        ))}
      </select>
      <div className="intro-actions">
        <button type="button" className="btn text" onClick={() => void act(() => api.deleteIntro(intro.id), 'Introduction removed. The person is still in your graph.')}>
          Remove
        </button>
      </div>
    </article>
  );
}

function NewIntroDialog({ onClose, onCreated }: { onClose: () => void; onCreated: () => void }) {
  const { api, toast, refreshStats } = useApp();
  const [person, setPerson] = useState('');
  const [connector, setConnector] = useState('');
  const [paths, setPaths] = useState<HowConnected['paths'] | null>(null);
  const [stage, setStage] = useState<IntroStage>('draft');
  const [note, setNote] = useState('');
  const [followUp, setFollowUp] = useState('');
  const [error, setError] = useState<string | null>(null);

  const ref = (s: string): number | string => (/^\d+$/.test(s.trim()) ? Number(s.trim()) : s.trim());

  return (
    <Dialog title="A thoughtful introduction starts here." onClose={onClose}>
      <form
        onSubmit={async (e) => {
          e.preventDefault();
          if (!person.trim()) return;
          setError(null);
          try {
            await api.createIntro({
              person: ref(person),
              connector: connector ? ref(connector) : null,
              stage,
              note: note.trim() || null,
              follow_up: followUp || null,
            });
            toast('Saved to introductions. Nothing was sent.');
            refreshStats();
            onCreated();
          } catch (err) {
            setError((err as Error).message);
          }
        }}
      >
        <label className="field">
          Who do you want to meet?
          <input value={person} onChange={(e) => setPerson(e.target.value)} required placeholder="name, email or id" />
        </label>
        <button
          type="button"
          className="btn text"
          disabled={!person.trim()}
          onClick={async () => {
            setError(null);
            try {
              const how = await api.connections(person.trim(), 5);
              setPaths(how.paths);
              if (!how.paths.length) setError('No mutual paths in your graph. You can still track a direct ask.');
            } catch (err) {
              setError((err as Error).message);
            }
          }}
        >
          <Icon name="route" />
          Suggest connectors
        </button>
        {paths && paths.length > 0 ? (
          <label className="field">
            Connector
            <select value={connector} onChange={(e) => setConnector(e.target.value)}>
              <option value="">Direct ask (no connector)</option>
              {paths.map((p) => (
                <option key={p.via.id} value={String(p.via.id)}>
                  {displayName(p.via)} · strength {p.strength.toFixed(2)}
                </option>
              ))}
            </select>
          </label>
        ) : (
          <label className="field">
            Connector (optional)
            <input value={connector} onChange={(e) => setConnector(e.target.value)} placeholder="who could introduce you" />
          </label>
        )}
        <div className="form-grid">
          <label className="field">
            Stage
            <select value={stage} onChange={(e) => setStage(e.target.value as IntroStage)}>
              {INTRO_STAGES.map((s) => (
                <option key={s} value={s}>
                  {STAGE_LABEL[s]}
                </option>
              ))}
            </select>
          </label>
          <label className="field">
            Follow-up date
            <input type="date" value={followUp} onChange={(e) => setFollowUp(e.target.value)} />
          </label>
        </div>
        <label className="field">
          Note
          <textarea value={note} onChange={(e) => setNote(e.target.value)} maxLength={4000} placeholder="what you'd like to connect about" />
        </label>
        <p>Ask the connector to check interest first. Write and send the message yourself, outside openstance.</p>
        {error && <div className="alert">{error}</div>}
        <div className="modal-actions">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button type="submit" className="btn primary">
            <Icon name="check" />
            Save to introductions
          </button>
        </div>
      </form>
    </Dialog>
  );
}
