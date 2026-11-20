import {useEffect, useState} from 'react';
import {call, type Action, type Row, sections, day} from './api';
import {Methods} from './Methods';
import {Page} from './Page';
import {Stroke, Enso} from './Ink';
function Entry({v, act, readonly}: {v: Row; act: Action; readonly: boolean}) {
  const [expanded, show] = useState(false),
    [edit, setEdit] = useState(false),
    [value, set] = useState(v.text),
    [removing, remove] = useState(false);
  return (
    <article className="entry">
      <div className="entry-line">
        <button
          className="entry-title"
          aria-expanded={expanded}
          onClick={() => show(!expanded)}
        >
          {v.text}
        </button>
        {(v.currentConfidence ?? v.confidence) >= 0.5 && (
          <Enso confidence={v.currentConfidence ?? v.confidence} size={27} />
        )}
      </div>
      {expanded && (
        <div className="evidence">
          <p className="small">
            {v.authored ? 'Written by you' : `Evidence on ${v.days} days`} ·{' '}
            {Math.round(100 * (v.currentConfidence ?? v.confidence))}%
            confidence
          </p>
          {v.evidence?.map((e: Row) => (
            <blockquote key={e.id}>
              {e.quote}
              <small>
                {e.source} · {day(e.date)}
              </small>
            </blockquote>
          ))}
          {!readonly &&
            (edit ? (
              <form
                onSubmit={async e => {
                  e.preventDefault();
                  await act('edit', {id: v.id, text: value});
                  setEdit(false);
                }}
              >
                <label>
                  Correct this entry
                  <textarea
                    value={value}
                    onChange={e => set(e.target.value)}
                    required
                  />
                </label>
                <button className="primary">Save correction</button>{' '}
                <button type="button" onClick={() => setEdit(false)}>
                  Cancel
                </button>
              </form>
            ) : (
              <div className="inline">
                <button
                  onClick={() => {
                    set(v.text);
                    setEdit(true);
                  }}
                >
                  Edit sentence
                </button>
                <button onClick={() => remove(!removing)}>
                  Strike through
                </button>
                {removing && (
                  <button onClick={() => act('delete', {id: v.id})}>
                    Confirm removal
                  </button>
                )}
              </div>
            ))}
        </div>
      )}
    </article>
  );
}
export function Scroll({
  data,
  act,
  teach,
}: {
  data: Row;
  act: Action;
  teach: () => void;
}) {
  const [version, select] = useState(''),
    [past, setPast] = useState<Row[]>([]),
    [adding, add] = useState(false),
    [draft, set] = useState(''),
    [section, choose] = useState('deciding');
  const versions = [...data.versions].reverse();
  const entries = version ? past : data.entries;
  useEffect(() => {
    let keep = true;
    if (version)
      call('version', {id: version}).then(v => {
        if (keep) setPast(v.entries);
      });
    return () => {
      keep = false;
    };
  }, [version]);
  return (
    <Page
      name="Scroll"
      margin={
        <>
          <button onClick={teach}>Teach a correction</button>
          {!version && (
            <button onClick={() => add(!adding)}>
              {adding ? 'Cancel' : 'Write an entry'}
            </button>
          )}
        </>
      }
    >
      {versions.length > 1 && (
        <label className="timeline">
          <span>{day(versions[0].date)}</span>
          <input
            aria-label="Scroll history"
            type="range"
            min={0}
            max={versions.length}
            value={
              version
                ? versions.findIndex(v => v.id === version)
                : versions.length
            }
            onChange={e => select(versions[+e.target.value]?.id || '')}
          />
          <span>{version ? 'Past' : 'Now'}</span>
        </label>
      )}
      {adding && !version && (
        <form
          className="insert"
          onSubmit={async e => {
            e.preventDefault();
            await act('create', {section, text: draft});
            set('');
            add(false);
          }}
        >
          <label>
            Section
            <select value={section} onChange={e => choose(e.target.value)}>
              {Object.entries(sections).map(([k, x]) => (
                <option key={k} value={k}>
                  {x}
                </option>
              ))}
            </select>
          </label>
          <label>
            Your sentence
            <textarea
              value={draft}
              required
              onChange={e => set(e.target.value)}
              autoFocus
            />
          </label>
          <button className="primary">Keep in the Scroll</button>
        </form>
      )}
      {!version && data.proposals.length > 0 && (
        <section>
          <h2>Waiting for your seal</h2>
          {data.proposals.map((v: Row) => (
            <article className="entry" key={v.id}>
              <div className="entry-line">
                <p className="entry-title">{v.text}</p>
                <button
                  className="seal-button"
                  aria-label="Sounds like me"
                  onClick={() => act('stamp', {id: v.id})}
                >
                  <Stroke points={data.mark} /> Sounds like me
                </button>
              </div>
              <details>
                <summary>Seen across {v.days} days</summary>
                {v.evidence.map((e: Row) => (
                  <blockquote key={e.id}>
                    {e.quote}
                    <small>
                      {e.source} · {day(e.date)}
                    </small>
                  </blockquote>
                ))}
              </details>
              <button
                className="quiet small"
                onClick={() => act('dismiss', {id: v.id})}
              >
                Not me
              </button>
            </article>
          ))}
        </section>
      )}
      {Object.entries(sections).map(([k, name]) => {
        const rows = entries.filter((v: Row) => v.section === k);
        return rows.length ? (
          <section key={k}>
            <h2>{name}</h2>
            {rows.map((v: Row) => (
              <Entry
                v={v}
                key={v.id + version}
                act={act}
                readonly={!!version}
              />
            ))}
          </section>
        ) : null;
      })}
      {!entries.length && !data.proposals.length && (
        <div className="empty">
          <h2>A place for what holds true.</h2>
          <p>
            Write a sentence about how you work, or let a reading bring a
            pattern here for your seal.
          </p>
        </div>
      )}
      {!version && <Methods skills={data.skills} act={act} />}
    </Page>
  );
}
