import { useState } from 'react';
import type { ShortlistEntry } from '../api/types';
import { useApp } from '../app/context';
import { href } from '../app/router';
import { Avatar, Empty, ErrorNote, Loading, PageHead, ScoreBar } from '../components/bits';
import { Icon } from '../components/Icon';
import { displayName, shortDate } from '../lib/format';
import { useLoad } from '../lib/useLoad';

export function Shortlist() {
  const { api } = useApp();
  const list = useLoad(() => api.shortlist(), [api]);
  return (
    <>
      <PageHead
        title="People worth a conversation."
        sub="A shortlist you own. Turn a good match into a meaningful introduction. Kept on this device; never shared or sent."
        aside={
          <a className="btn" href="#/discover">
            <Icon name="search" />
            Find people
          </a>
        }
      />
      <ErrorNote error={list.error} />
      {list.loading && !list.data && <Loading what="Loading your shortlist" />}
      {list.data && !list.data.length && (
        <Empty
          title="Your next great conversation starts here."
          icon="bookmark"
          action={
            <a className="btn soft" href="#/discover">
              Browse people <Icon name="arrow" />
            </a>
          }
        >
          Save people from search or their page to build your shortlist.
        </Empty>
      )}
      {list.data?.map((e, i) => (
        <ShortlistRow key={e.person_id} entry={e} index={i} onChanged={list.reload} />
      ))}
    </>
  );
}

function ShortlistRow({ entry, index, onChanged }: { entry: ShortlistEntry; index: number; onChanged: () => void }) {
  const { api, toast, refreshStats } = useApp();
  const [editing, setEditing] = useState(false);
  const [note, setNote] = useState(entry.note ?? '');
  const p = entry.person;
  const name = p ? displayName(p) : `Person #${entry.person_id} (no longer in your graph)`;

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
    <article className="person-card">
      <div className="person-head">
        <Avatar name={p?.display_name} tone={index} />
        <div>
          <a className="person-name" href={href({ name: 'person', id: String(entry.person_id) })}>
            {name}
          </a>
          <div className="person-role">{[p?.title, p?.company].filter(Boolean).join(' · ') || 'Title and company unknown'}</div>
          <div className="person-location">
            <Icon name="clock" />
            Added {shortDate(entry.added_at)}
          </div>
        </div>
        {p?.score !== undefined && <ScoreBar score={p.score} />}
      </div>
      {editing ? (
        <form
          className="note-form"
          onSubmit={(e) => {
            e.preventDefault();
            setEditing(false);
            void act(() => api.setShortlistNote(entry.person_id, note.trim() || null), 'Note saved.');
          }}
        >
          <textarea aria-label="Note" value={note} onChange={(e) => setNote(e.target.value)} maxLength={4000} autoFocus />
          <button type="submit" className="btn primary" aria-label="Save note">
            Save
          </button>
          <button type="button" className="btn" onClick={() => setEditing(false)}>
            Cancel
          </button>
        </form>
      ) : (
        <p className="reason">{entry.note ?? <span className="muted">No note yet.</span>}</p>
      )}
      <div className="card-bottom">
        <span className="path-label">
          <button type="button" className="btn text" aria-label="Edit note" onClick={() => setEditing(true)}>
            <Icon name="edit" />
            Note
          </button>
          <button
            type="button"
            className="btn text"
            onClick={() => void act(() => api.removeFromShortlist(entry.person_id), `Removed ${name} from your shortlist.`)}
          >
            Remove from shortlist
          </button>
        </span>
        <button
          type="button"
          className="btn"
          onClick={() =>
            void act(
              () => api.createIntro({ person: entry.person_id, stage: 'draft' }),
              'Added to your introductions board. Nothing was sent.',
            )
          }
        >
          <Icon name="mail" />
          Track an intro
        </button>
      </div>
    </article>
  );
}
