import {useState} from 'react';
import {BlindReadings} from './BlindReadings';
import {type Row, type Action, day} from './api';
export function Mirror({data, act}: {data: Row; act: Action}) {
  const [reply, set] = useState(''),
    [focus, select] = useState(''),
    [ruleId, selectRule] = useState('');
  const scores: Row[] = data.scores;
  const today = new Date();
  const weekStart = new Date(today);
  weekStart.setDate(today.getDate() - 6);
  weekStart.setHours(0, 0, 0, 0);
  const thisWeek = (date: string) => {
    const when = Date.parse(date);
    return when >= weekStart.getTime() && when <= today.getTime();
  };
  const week = scores.filter(
    s =>
      thisWeek(s.date) && Number.isInteger(s.grade) && s.category !== 'ghost',
  );
  const wins = week.filter(v => v.grade >= 2).length;
  const series = Array.from({length: 7}, (_, i) => {
    const point = new Date(today);
    point.setDate(today.getDate() - (6 - i));
    const date = point.toLocaleDateString();
    const vs = week.filter(v => new Date(v.date).toLocaleDateString() === date);
    return vs.length ? vs.filter(v => v.grade >= 2).length / vs.length : null;
  });
  const byId = new Map(scores.map(v => [v.id, v]));
  const misses: Row[] = (data.surprises || []).slice(0, 3);
  const recorded = week.filter(v => typeof v.probability === 'number');
  const brier = recorded.length
    ? recorded.reduce(
        (sum, v) => sum + (v.probability - Number(v.grade >= 2)) ** 2,
        0,
      ) / recorded.length
    : null;
  const ghost = scores.filter(
    v => v.category === 'ghost' && !!v.status && thisWeek(v.date),
  );
  const changes: Row[] =
    today.getDay() === 0
      ? data.entries
          .filter((v: Row) => v.text && thisWeek(v.updated))
          .sort(
            (a: Row, b: Row) => Date.parse(b.updated) - Date.parse(a.updated),
          )
      : [];
  const latest = changes[0];
  const observed =
    week.find(v => v.grade < 2 && v.actual) || week.find(v => v.actual);
  const quote = (value: string) =>
    '“' + value.trim().replace(/[.!?。！？]?$/u, end => end || '.') + '”';
  return (
    <main className="page mirror">
      <div className="folio">鏡 · The past seven days</div>
      <h1>The Mirror</h1>
      <div className="number">
        {week.length ? Math.round((100 * wins) / week.length) + '%' : '—'}
      </div>
      <p>
        {week.length
          ? `${wins} of ${week.length} scored guesses matched.`
          : 'Your first scored guess will appear here.'}
      </p>
      {week.length > 0 && (
        <svg
          className="trend"
          viewBox="0 0 320 50"
          role="img"
          aria-label="Daily accuracy, last seven days"
        >
          {series.map((v, i) =>
            v === null ? null : (
              <g key={i}>
                <circle
                  cx={4 + i * 51}
                  cy={44 - v * 37}
                  r="3"
                  fill="currentColor"
                />
                {i > 0 && series[i - 1] !== null && (
                  <line
                    x1={4 + (i - 1) * 51}
                    y1={44 - series[i - 1]! * 37}
                    x2={4 + i * 51}
                    y2={44 - v * 37}
                    stroke="currentColor"
                    strokeWidth="2"
                  />
                )}
              </g>
            ),
          )}
        </svg>
      )}
      <div className="rows">
        {[
          ['words', 'Your words'],
          ['decisions', 'Your decisions'],
          ['agents', 'Answers for you'],
        ].map(([k, n]) => {
          const rows = week.filter(v => v.category === k);
          return (
            <div className="row" key={k}>
              <span>{n}</span>
              <span className="mono">
                {rows.length
                  ? rows.filter(v => v.grade >= 2).length + ' / ' + rows.length
                  : '—'}
              </span>
            </div>
          );
        })}
      </div>
      <BlindReadings scores={scores} />
      {misses.length > 0 && (
        <section>
          <h2>Surprises</h2>
          {misses.map(v => {
            const observations = (v.members as string[]).map(
              id => byId.get(id)!,
            );
            const first = observations[0];
            return (
              <article className="entry" key={v.id}>
                {v.ruleId ? (
                  <>
                    <p>“{v.ruleText}”</p>
                    <small>
                      Sumi found possible exceptions on {v.days} days, from{' '}
                      {day(
                        observations[observations.length - 1].observedAt ||
                          observations[observations.length - 1].date,
                      )}{' '}
                      to {day(first.observedAt || first.date)}.
                    </small>
                    <details>
                      <summary>
                        Read the {observations.length} observations
                      </summary>
                      {observations.map(o => (
                        <blockquote key={o.id}>
                          <small>
                            {day(o.observedAt || o.date)} · {o.question}
                          </small>
                          {o.actual}
                          <small>Sumi guessed · {o.guess}</small>
                        </blockquote>
                      ))}
                    </details>
                  </>
                ) : (
                  <>
                    <p>“{first.guess}”</p>
                    <small>You actually said · {first.actual}</small>
                  </>
                )}
                {focus !== v.id ? (
                  <button
                    onClick={() => {
                      select(v.id);
                      set('');
                      selectRule(v.ruleId || '');
                    }}
                  >
                    Change or exception?
                  </button>
                ) : (
                  <>
                    {data.entries.some((r: Row) =>
                      v.ruleIds?.includes(r.id),
                    ) && (
                      <label>
                        Apply this to
                        <select
                          value={ruleId}
                          onChange={e => selectRule(e.target.value)}
                        >
                          <option value="">This observation</option>
                          {data.entries
                            .filter((r: Row) => v.ruleIds?.includes(r.id))
                            .map((r: Row) => (
                              <option key={r.id} value={r.id}>
                                {r.text}
                              </option>
                            ))}
                        </select>
                      </label>
                    )}
                    <label>
                      {v.ruleId
                        ? 'How would you word the change or exception?'
                        : 'What was different?'}
                      <input
                        value={reply}
                        onChange={e => set(e.target.value)}
                      />
                    </label>
                    <div className="inline">
                      {['An exception', 'A change'].map((n, i) => (
                        <button
                          key={n}
                          disabled={!reply.trim()}
                          onClick={async () => {
                            await act('surprise', {
                              id: v.id,
                              text: reply,
                              change: !!i,
                              ruleId: ruleId || undefined,
                            });
                            select('');
                          }}
                        >
                          {n}
                        </button>
                      ))}
                      {v.ruleId && (
                        <button
                          onClick={async () => {
                            await act('surprise', {
                              id: v.id,
                              ruleId: v.ruleId,
                              keep: true,
                            });
                            select('');
                          }}
                        >
                          Keep the belief as written
                        </button>
                      )}
                    </div>
                  </>
                )}
              </article>
            );
          })}
        </section>
      )}
      {today.getDay() === 0 &&
        (latest || week.length > 0 || ghost.length > 0) && (
          <section aria-label="Sunday note">
            <h2>Sunday note</h2>
            <p>
              {latest ? (
                <>
                  Your Scroll has{' '}
                  {changes.length === 1
                    ? 'one new or revised entry'
                    : changes.length + ' new or revised entries'}{' '}
                  this week. The latest reads {quote(latest.text)}{' '}
                </>
              ) : (
                <>None of the current Scroll entries changed this week. </>
              )}
              {week.length ? (
                <>
                  Sumi matched {wins} of {week.length} scored{' '}
                  {week.length === 1 ? 'guess' : 'guesses'}.
                </>
              ) : (
                <>There are no scored guesses for the week yet.</>
              )}
              {!latest &&
                (ghost.length ? (
                  <>
                    {' '}
                    You kept {
                      ghost.filter(v => v.status === 'accepted').length
                    }{' '}
                    of {ghost.length} writing suggestions.
                  </>
                ) : observed ? (
                  <>
                    {' '}
                    One reply it {observed.grade < 2
                      ? 'missed'
                      : 'matched'} was {quote(observed.actual)}
                  </>
                ) : (
                  <> Each scored guess is in your prediction record.</>
                ))}
            </p>
          </section>
        )}
      <details>
        <summary>How the score is measured</summary>
        <p className="small">
          Replies match when the intent is the same. Decisions use your verdict.
          Each blind reading records its guesses before opening the observed
          replies.
        </p>
        {brier !== null && (
          <p>
            Brier score · {brier.toFixed(3)} across {recorded.length}{' '}
            predictions with probabilities.
          </p>
        )}
        {ghost.length > 0 && (
          <p>
            Ghost ink · {ghost.filter(v => v.grade === 3).length} accepted of{' '}
            {ghost.length} shown.
          </p>
        )}
      </details>
      <details>
        <summary>Where it ran</summary>
        {data.catalogUpdated && (
          <p>Model recommendations updated {day(data.catalogUpdated)}.</p>
        )}
        <p className="small">
          {data.model?.runtime === 'ChatGPT' ? 'ChatGPT plan' : 'Local model'} ·{' '}
          {data.model?.name || 'Unselected'}
        </p>
        {data.routes.map((r: Row) => (
          <p key={r.id}>
            {day(r.date)} · {r.provider} · {r.job}
          </p>
        ))}
        {data.tuning.map((r: Row) => (
          <p key={r.id}>
            {day(r.date)} · {r.cases} separate tuning cases · {r.selected}{' '}
            prompt
          </p>
        ))}
        {data.backgroundIssue && <p>{data.backgroundIssue}</p>}
      </details>
      <details>
        <summary>Prediction record</summary>
        {scores.length ? (
          scores.slice(0, 60).map(v => (
            <div className="record" key={v.id}>
              <small>
                {day(v.date)} · {v.model || 'local'} · {v.scope}
              </small>
              <p>{v.guess || 'Ghost ink'}</p>
              {v.category === 'ghost' && v.status === 'edited' && (
                <p>You wrote · {v.actual}</p>
              )}
              <small>
                {v.category === 'ghost'
                  ? {
                      shown: 'Shown',
                      edited: 'You wrote your own ending',
                      accepted: 'Accepted with Tab',
                      dismissed: 'Dismissed',
                    }[v.status as string] ||
                    (v.grade === 3 ? 'Accepted with Tab' : 'Passed over')
                  : ['Different', 'Partial', 'Same intent', 'Matched'][v.grade]}
              </small>
            </div>
          ))
        ) : (
          <p className="small">No scored predictions yet.</p>
        )}
      </details>
    </main>
  );
}
