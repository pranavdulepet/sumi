import {useState} from 'react';
import {call, type Row} from './api';
const percent = new Intl.NumberFormat(undefined, {style: 'percent', maximumFractionDigits: 1});

export function NotebookData({root}: {root: string}) {
  const [confirm, setConfirm] = useState(false),
    [text, write] = useState(''),
    [status, note] = useState(''),
    [working, busy] = useState(false),
    [metrics, show] = useState<Row | null>(null);
  const [through, setThrough] = useState('');
  async function readMeasures(day?: string) {
    const result = await call('notebookMeasures', day ? {day} : {});
    show(result);
    setThrough(result.through);
  }
  async function run(work: () => Promise<void>) {
    busy(true);
    note('');
    try {
      await work();
    } catch (e) {
      note(String(e));
    } finally {
      busy(false);
    }
  }
  return (
    <details>
      <summary>Your notebook</summary>
      <p className="small">{root}</p>
      <button
        disabled={working}
        onClick={() =>
          run(async () => {
            const saved = await call('exportNotebook');
            if (saved) note('Saved to ' + saved);
          })
        }
      >
        Export a readable copy
      </button>
      <button
        disabled={working}
        onClick={() => run(() => readMeasures(through || undefined))}
      >
        Review local measures
      </button>
      {metrics && (
        <div>
          <form
            onSubmit={e => {
              e.preventDefault();
              void run(() => readMeasures(through));
            }}
          >
            <label>
              Two weeks ending{' '}
              <input
                type="date"
                value={through}
                max={metrics.today}
                required
                onChange={e => setThrough(e.target.value)}
              />
            </label>{' '}
            <button
              disabled={working || !through || through === metrics.through}
            >
              Apply
            </button>
          </form>
          <table>
            <caption>Seven-day periods through {metrics.through}</caption>
            <thead>
              <tr>
                <th>Week starting</th>
                <th>Days used</th>
                <th>Replies</th>
                <th>Clarifications¹</th>
                <th>Rejected</th>
                <th>Deep</th>
              </tr>
            </thead>
            <tbody>
              {metrics.weeks.map((w: Row) => (
                <tr key={w.week}>
                  <td>{w.week}</td>
                  <td>{w.days.length}</td>
                  <td>{w.turns}</td>
                  <td>
                    {w.turns
                      ? `${w.clarifications} of ${w.turns} (${percent.format(w.clarifications / w.turns)})`
                      : 'No replies'}
                  </td>
                  <td>{w.rejections}</td>
                  <td>{w.deep}</td>
                </tr>
              ))}
            </tbody>
          </table>
          <small>
            ¹ Follow-ups beginning with a clarification such as “I meant.”
            Rejected counts corrections and regenerations made during each
            period.
          </small>
          <p className="small">Across this notebook</p>
          <p>
            {metrics.eligibleSkills
              ? metrics.reusedSkills +
                ' of ' +
                metrics.eligibleSkills +
                ' learned skills were reused in their first month.'
              : 'Skills need a month of history to measure reuse.'}
          </p>
          <p>
            {metrics.memoryChecks
              ? metrics.confirmedNotes +
                ' of ' +
                metrics.memoryChecks +
                ' rechecked memories agree with a later statement.'
              : 'Memory precision needs a later statement that confirms or contradicts a note.'}
          </p>
          {metrics.dreamTrial?.cases && (
            <small>
              Last memory trial · {metrics.dreamTrial.cases} held-out replies ·{' '}
              {metrics.dreamTrial.baseline} points before,{' '}
              {metrics.dreamTrial.candidate} after.
            </small>
          )}
        </div>
      )}
      <button onClick={() => setConfirm(!confirm)}>Remove this notebook</button>
      {confirm && (
        <form
          onSubmit={e => {
            e.preventDefault();
            void run(async () => {
              await call('trashNotebook', {confirm: text});
            });
          }}
        >
          <p>
            The entire folder at {root}, including files you added, will move to
            the Mac’s Trash. Sumi will reopen with a blank notebook.
          </p>
          <label>
            Type “Move notebook to Trash” to confirm
            <input value={text} onChange={e => write(e.target.value)} />
          </label>
          <button disabled={working || text !== 'Move notebook to Trash'}>
            Move notebook to Trash
          </button>
          <button
            type="button"
            onClick={() => {
              write('');
              setConfirm(false);
            }}
          >
            Cancel
          </button>
        </form>
      )}
      {status && <p role="status">{status}</p>}
    </details>
  );
}
