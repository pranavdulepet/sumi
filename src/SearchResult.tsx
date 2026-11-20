import {useState} from 'react';
import {type Row, type Action, day} from './api';

export function SearchResult({
  turn,
  sheet,
  act,
}: {
  turn: Row;
  sheet: string;
  act: Action;
}) {
  const [busy, working] = useState(false);
  const web = turn.web;
  if (!web) return null;
  if (web.status === 'ready' || web.status === 'retry')
    return (
      <div className="search-proposal">
        <small>Public search · Exa</small>
        <p>{web.query}</p>
        <button
          disabled={busy}
          onClick={async () => {
            working(true);
            try {
              await act('searchWeb', {sheet, turn: turn.id, query: web.query});
            } finally {
              working(false);
            }
          }}
        >
          {busy
            ? 'Reading sources…'
            : web.status === 'retry'
              ? 'Try search again'
              : 'Search the web'}
        </button>
      </div>
    );
  if (!web.sources?.length) return null;
  function source(index: number) {
    const row = web.sources[index];
    return (
      <button
        className="source-link"
        key={row.url}
        onClick={() => act('openWebSource', {sheet, turn: turn.id, index})}
      >
        {row.title} ↗{' '}
        <small>{new URL(row.url).hostname.replace(/^www\./, '')}</small>
      </button>
    );
  }
  return (
    <div className="source-list">
      {Number.isInteger(web.pick) && source(web.pick)}
      <details>
        <summary>Sources checked · {day(web.checkedAt)}</summary>
        {web.sources.map((_: Row, i: number) => source(i))}
      </details>
    </div>
  );
}
