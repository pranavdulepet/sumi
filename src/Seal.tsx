import {Providers} from './Providers';
import {useState} from 'react';
import {type Row, type Action, sections, day} from './api';
import {Models, Sources} from './Onboarding';
function AnswerReviews({
  agent,
  log,
  update,
}: {
  agent: Row;
  log: Row[];
  update: (settings: Row) => Promise<any>;
}) {
  const automatic = log.filter(
    row =>
      row.agent === agent.id &&
      row.tool === 'sumi_answer' &&
      ['answered', 'reviewed'].includes(row.status) &&
      typeof row.shared?.answer === 'string',
  );
  if (!automatic.length) return null;
  const reviewed = automatic.filter(row => typeof row.human === 'string');
  const corrected = (rows: Row[]) =>
    rows.filter(row => row.human !== row.shared.answer).length;
  const recent = reviewed.slice(0, 60);
  const next = Math.round(agent.threshold * 100) + 1;
  const above = recent.filter(
    row => Number.isFinite(row.confidence) && row.confidence >= next / 100,
  );
  const suggest =
    next < 100 &&
    above.length >= 20 &&
    corrected(recent) * 20 < recent.length &&
    corrected(above) * 20 < above.length;
  return (
    <div>
      <p className="small">
        {reviewed.length} of {automatic.length} automatic answers reviewed ·{' '}
        {corrected(reviewed)} corrected.
      </p>
      {suggest && (
        <>
          <p className="small">
            Among recent reviews at {next}% confidence or higher, you kept{' '}
            {above.length - corrected(above)} of {above.length} answers.
          </p>
          <button onClick={() => update({threshold: next / 100})}>
            Raise minimum to {next}%
          </button>
        </>
      )}
    </div>
  );
}
function Agent({a, act, log}: {a: Row; act: Action; log: Row[]}) {
  const [remove, ask] = useState(false);
  const update = (x: Row) => act('policy', {...a, ...x});
  if (a.linkState) {
    const removing = a.linkState === 'revoking';
    return (
      <div className="row">
        <span>
          {a.name}
          <small>
            {removing
              ? 'Access revoked. Configuration cleanup is pending.'
              : 'Connection setup is pending.'}
          </small>
        </span>
        <button
          onClick={() =>
            (removing
              ? act('disconnect', {id: a.id})
              : act('connect', {name: a.name})
            ).catch(() => {})
          }
        >
          {removing ? 'Finish removal' : 'Finish connection'}
        </button>
      </div>
    );
  }
  const instructions: Record<string, string> = {
    Read: 'Before choosing a default for me, call sumi_context for the relevant preferences.',
    Ask: 'Before asking me a preference question, call sumi_predict and use its guess as advice.',
    'Answer for me':
      'Before asking me a preference question, call sumi_answer and use its answer or wait for my reply.',
  };
  return (
    <details>
      <summary>
        {a.name}
        <span className="summary-end">{a.permission}</span>
      </summary>
      <label>
        Permission
        <select
          aria-label="Permission"
          value={a.permission}
          onChange={e => update({permission: e.target.value})}
        >
          {['Read', 'Ask', 'Answer for me'].map(p => (
            <option key={p}>{p}</option>
          ))}
        </select>
      </label>
      <fieldset>
        <legend>Visible sections</legend>
        {Object.entries(sections).map(([k, label]) => (
          <label className="check" key={k}>
            <input
              type="checkbox"
              checked={a.sections.includes(k)}
              onChange={e =>
                update({
                  sections: e.target.checked
                    ? [...a.sections, k]
                    : a.sections.filter((v: string) => v !== k),
                })
              }
            />
            {label}
          </label>
        ))}
      </fieldset>
      {a.permission === 'Answer for me' && (
        <label>
          Minimum confidence · {Math.round(a.threshold * 100)}%
          <input
            type="range"
            min=".80"
            max="1"
            step=".01"
            value={a.threshold}
            onChange={e => update({threshold: Number(e.target.value)})}
          />
        </label>
      )}
      {a.permission === 'Answer for me' && (
        <AnswerReviews agent={a} log={log} update={update} />
      )}
      <label>
        Line for {a.name}’s instructions
        <textarea
          readOnly
          rows={2}
          value={instructions[a.permission]}
          onFocus={e => e.currentTarget.select()}
        />
      </label>
      <small>MCP server · {a.serverName || 'sumi'}</small>
      <div className="inline">
        <button onClick={() => act('connect', {name: a.name}).catch(() => {})}>
          Repair connection
        </button>
        <button
          onClick={() =>
            remove ? act('disconnect', {id: a.id}).catch(() => {}) : ask(true)
          }
        >
          {remove ? 'Confirm revocation' : 'Revoke access'}
        </button>
        {remove && <button onClick={() => ask(false)}>Cancel</button>}
      </div>
    </details>
  );
}
export function ConnectAgents({
  data,
  act,
  names = ['Claude Code', 'Codex', 'Claude Desktop', 'Cursor'],
}: {
  data: Row;
  act: Action;
  names?: string[];
}) {
  const [connect, set] = useState('');
  return (
    <>
      {' '}
      <div className="inline">
        {names
          .filter(n => !data.agents.some((a: Row) => a.name === n))
          .map(n => (
            <button className="text-action" key={n} onClick={() => set(n)}>
              {n} ＋
            </button>
          ))}
      </div>
      {connect && (
        <div className="insert">
          <h3>Connect {connect}?</h3>
          <p>
            Sumi will add a local MCP server to its configuration. It starts
            with read access to decisions, writing, words and goals. Restart the
            agent to use it.
          </p>
          <div className="inline">
            <button
              className="primary"
              onClick={async () => {
                await act('connect', {name: connect});
                set('');
              }}
            >
              Connect {connect}
            </button>
            <button onClick={() => set('')}>Cancel</button>
          </div>
        </div>
      )}
    </>
  );
}
export function Seal({
  data,
  act,
  focusWaiting,
}: {
  data: Row;
  act: Action;
  focusWaiting: boolean;
}) {
  const [busy, working] = useState(false),
    [answer, write] = useState(''),
    [logId, select] = useState(''),
    [justAnswered, confirmAnswer] = useState('');
  const waiting = data.log.filter((row: Row) => row.status === 'waiting');
  const isCheck = (row: Row) =>
    row.tool === 'sumi_answer' && ['served', 'checked'].includes(row.status);
  const checks: Row[] = data.log.filter(isCheck);
  const recent = data.log.filter(
    (row: Row) => row.status !== 'waiting' && !isCheck(row),
  );
  const [checkCount, showMoreChecks] = useState(20);
  const [historyCount, showMore] = useState(60);
  const digest: Row[] = [...waiting, ...recent.slice(0, historyCount)];
  return (
    <main className="page">
      <div className="folio">印 · Permissions</div>
      <h1>The Seal</h1>
      <section>
        <h2>Your agents</h2>
        {data.agents.map((a: Row) => (
          <Agent a={a} key={a.id} act={act} log={data.log} />
        ))}
        {!data.agents.length && (
          <p className="muted">
            Choose an agent to give it a reading of your Scroll.
          </p>
        )}
        <ConnectAgents data={data} act={act} />
      </section>
      <section>
        <h2>Morning digest</h2>
        {!digest.length ? (
          <p className="muted">
            Questions and answers from your agents will appear here.
          </p>
        ) : (
          digest.map((v: Row) => (
            <details
              key={v.id}
              open={
                v.status === 'waiting' || v.id === justAnswered || undefined
              }
            >
              <summary>
                {v.question}
                <small>
                  {v.name} · {v.status} · {day(v.date)}
                </small>
              </summary>
              <p>{v.human || v.shared?.answer || v.shared?.guess}</p>
              {v.shared?.entries?.map((e: Row) => (
                <p key={e.id}>{e.text}</p>
              ))}
              {v.status === 'closed' && (
                <p>Access was revoked before you answered.</p>
              )}
              {v.status === 'answered' &&
                v.tool === 'sumi_answer' &&
                !v.human &&
                v.shared?.answer &&
                logId !== v.id && (
                  <button
                    onClick={async () => {
                      await act('respond', {id: v.id, answer: v.shared.answer});
                      confirmAnswer(v.id);
                    }}
                  >
                    That was right
                  </button>
                )}
              {v.tool === 'sumi_answer' &&
                ['waiting', 'answered', 'reviewed'].includes(v.status) &&
                (logId !== v.id ? (
                  <button
                    autoFocus={focusWaiting && v.id === waiting[0]?.id}
                    onClick={() => {
                      select(v.id);
                      write(v.human || '');
                    }}
                  >
                    {v.status === 'waiting'
                      ? 'Answer question'
                      : 'Correct answer'}
                  </button>
                ) : (
                  <form
                    onSubmit={async e => {
                      e.preventDefault();
                      await act('respond', {id: v.id, answer});
                      confirmAnswer(v.id);
                      select('');
                    }}
                  >
                    <label>
                      Your answer
                      <input
                        required
                        value={answer}
                        onChange={e => write(e.target.value)}
                      />
                    </label>
                    <button>Keep answer</button>
                  </form>
                ))}
            </details>
          ))
        )}
        {recent.length > historyCount && (
          <button className="small" onClick={() => showMore(n => n + 60)}>
            Earlier activity
          </button>
        )}
        {checks.length > 0 && (
          <details>
            <summary>Agent receipt history</summary>
            {checks.slice(0, checkCount).map(r => (
              <p className="small" key={r.id}>
                {r.name} · {day(r.date)} ·{' '}
                {r.shared?.answer || 'Still waiting for your answer.'}
              </p>
            ))}
            {checks.length > checkCount && (
              <button
                className="small"
                onClick={() => showMoreChecks(n => n + 20)}
              >
                Earlier receipts
              </button>
            )}
          </details>
        )}
      </section>
      <section>
        <details>
          <summary>Sources and readings</summary>
          <Sources
            sources={data.sources}
            request={data.requestedExport}
            act={act}
            confirmRemove
          />
          <button
            className="primary"
            disabled={busy}
            onClick={async () => {
              working(true);
              try {
                await act('read');
              } finally {
                working(false);
              }
            }}
          >
            {busy ? 'Reading…' : 'Read selected sources'}
          </button>
        </details>
        <details>
          <summary>Local intelligence</summary>
          <Models model={data.model} act={act} />
          {data.catalogConfigured && (
            <label className="check">
              <input
                type="checkbox"
                checked={!data.catalogOff}
                onChange={e => act('catalogOff', {value: !e.target.checked})}
              />
              Check for model updates weekly
            </label>
          )}
        </details>
        <Providers data={data} act={act} />
        <details>
          <summary>Learning preferences</summary>
          <label className="check">
            <input
              type="checkbox"
              checked={!!data.autoStamp}
              onChange={e => act('autoStamp', {value: e.target.checked})}
            />
            Automatically stamp well-supported writing habits
          </label>
          <label className="check">
            <input
              type="checkbox"
              checked={!data.marginOff}
              onChange={e => act('marginOff', {value: !e.target.checked})}
            />
            Show notes from my approved goals
          </label>
        </details>
        <details>
          <summary>Notebook on disk</summary>
          <small>{data.root}</small>
          {data.globalShortcutReady === false && (
            <p className="small">
              macOS could not register ⌥Space. Use New Desk Sheet in Sumi’s menu
              bar menu, or press ⌥Space inside Sumi.
            </p>
          )}
          <button onClick={() => act('openFolder')}>
            Open notebook folder ↗
          </button>
        </details>
      </section>
    </main>
  );
}
