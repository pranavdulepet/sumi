import {ExportArrival} from './ExportArrival';
import {ConnectAgents} from './Seal';
import {Think} from './Thinking';
import {FirstCorrection} from './FirstCorrection';
import {useState, useEffect} from 'react';
import {call, type Row, type Action} from './api';
import {MarkPad, Stroke, Enso} from './Ink';
export function Sources({
  sources,
  request,
  act,
  confirmRemove = false,
}: {
  sources: Row[];
  request?: Row;
  act: Action;
  confirmRemove?: boolean;
}) {
  const [showMessages, consent] = useState(false),
    [exports, load] = useState<Row[] | null>(null);
  const [found, set] = useState<Row[]>([]),
    [remove, ask] = useState('');
  useEffect(() => {
    call('discover')
      .then(set)
      .catch(() => {});
  }, []);
  return (
    <>
      <div className="rows">
        {found
          .filter(v => !sources.some(s => s.id === v.id))
          .map(v => (
            <button
              key={v.id}
              className="row"
              onClick={() => act('addSource', {id: v.id})}
            >
              <span>
                {v.label}
                <small>{v.count} local session files</small>
              </span>
              <span>Add ＋</span>
            </button>
          ))}
        {sources.map(s => (
          <div className="row" key={s.id}>
            <span className="export-name">
              {s.label}
              <small>Included in your reading</small>
            </span>
            <button onClick={() => ask(s.id)}>Remove</button>
            {remove === s.id && (
              <div className="sub">
                <p>
                  Forget this source and its learned history? Original files
                  stay on your Mac.
                </p>
                <button
                  onClick={async () => {
                    await act('forget', {id: s.id});
                    ask('');
                  }}
                >
                  Forget source
                </button>{' '}
                <button onClick={() => ask('')}>Cancel</button>
              </div>
            )}
          </div>
        ))}
      </div>
      <div className="inline">
        <button onClick={() => act('pickSource', {kind: 'export'})}>
          Import chat export
        </button>
        {confirmRemove && (
          <button onClick={() => act('pickSource', {kind: 'folder'})}>
            Choose notes folder
          </button>
        )}
        {!request && (
          <button onClick={() => void act('exportLink').catch(() => {})}>
            Request ChatGPT history ↗
          </button>
        )}
        <button
          onClick={() =>
            void act('exports')
              .then(load)
              .catch(() => {})
          }
        >
          Find downloaded exports
        </button>
      </div>
      {!request && (
        <p className="small">
          Requesting history lets Sumi look for the ZIP in Downloads. You choose
          whether to add it.
        </p>
      )}
      <ExportArrival request={request} act={act} />
      {exports?.length === 0 && (
        <p className="small" role="status">
          No new chat exports found in Downloads.
        </p>
      )}
      {exports
        ?.filter(
          v =>
            !sources.some(s => s.path === v.path) &&
            !request?.offers?.some((r: Row) => r.path === v.path),
        )
        .map(v => (
          <button
            className="row export-name"
            key={v.path}
            onClick={() =>
              void act('acceptExport', {path: v.path})
                .then(() => load(null))
                .catch(() => {})
            }
          >
            Add {v.name} to Sumi
          </button>
        ))}
      {confirmRemove && !sources.some(s => s.id === 'messages') && (
        <details>
          <summary>Include sent Messages</summary>
          <p className="small">
            Reads your sent texts and the preceding message in each
            conversation. This source requires Full Disk Access and stays local.
          </p>
          <label className="check">
            <input
              type="checkbox"
              checked={showMessages}
              onChange={e => consent(e.target.checked)}
            />
            Include my sent Messages
          </label>
          <button
            disabled={!showMessages}
            onClick={() => act('messages', {consent: true})}
          >
            Add Messages
          </button>
        </details>
      )}
    </>
  );
}
export function Onboarding({
  data,
  act,
  finish,
  refresh,
}: {
  data: Row;
  act: Action;
  finish: () => void;
  refresh: () => Promise<void>;
}) {
  const [choice, quiz] = useState<Row | null>(null),
    [last, result] = useState<Row | null>(null);
  const [chosenStep, go] = useState(
    data.model && data.mark ? 2 : data.mark ? 1 : 0,
  );
  const [busy, setBusy] = useState(false);
  const [err, error] = useState('');
  const [notify, setNotify] = useState(data.firstReading?.notify === true);
  const [sentence, setSentence] = useState('');
  const reading: Row | null = data.reading;
  const job = data.firstReading;
  const step = reading ? 3 : job ? 2 : chosenStep;
  const running = job?.status === 'running' && !reading;
  useEffect(() => {
    if (!running) return;
    let live = true;
    let checking = false;
    async function check() {
      if (checking || document.hidden) return;
      checking = true;
      try {
        const status = await call('firstReadingProgress');
        if (!live) return;
        if (status.job?.status !== 'running') await refresh();
        else setSentence(status.progress?.sentence || '');
      } catch (e) {
        if (live) error(String(e));
      } finally {
        checking = false;
      }
    }
    void check();
    const timer = window.setInterval(check, 1500);
    document.addEventListener('visibilitychange', check);
    return () => {
      live = false;
      window.clearInterval(timer);
      document.removeEventListener('visibilitychange', check);
    };
  }, [running, refresh]);
  async function read() {
    setBusy(true);
    error('');
    setSentence('');
    try {
      await act('beginFirstReading', {notify});
    } catch (e) {
      error(String(e));
    } finally {
      setBusy(false);
    }
  }
  if (step === 2 && !job)
    return (
      <main className="first-lesson">
        <FirstCorrection data={data} act={act} />
      </main>
    );
  return (
    <main className={'welcome stage-' + step}>
      {step === 0 ? (
        <>
          <h1>
            A mark of your own<span className="punct">.</span>
          </h1>
          <p className="lede">Draw one stroke. It becomes your seal.</p>
          <MarkPad
            save={async p => {
              await act('mark', {points: p});
              go(1);
            }}
          />
        </>
      ) : step === 1 ? (
        <Think act={act} data={data} next={() => go(2)} />
      ) : step === 2 ? (
        <>
          <h1>
            {running || busy ? 'Reading your words.' : 'Your earlier reading.'}
          </h1>
          <p className="lede">
            {running || busy
              ? 'You can close this window. If you quit Sumi, the reading resumes when you return.'
              : 'Continue the reading you previously selected.'}
          </p>
          {running || busy ? (
            <div className="reading" aria-live="polite">
              <Enso size={83} />
              {sentence && (
                <p className="reading-sentence" key={sentence}>
                  {sentence}
                </p>
              )}
            </div>
          ) : (
            <>
              {data.sources.length > 0 && (
                <label className="check">
                  <input
                    type="checkbox"
                    checked={notify}
                    onChange={e => setNotify(e.target.checked)}
                  />
                  Notify me when the reading is ready
                </label>
              )}
              <button className="primary" onClick={read}>
                {job?.status === 'failed'
                  ? 'Retry the first reading'
                  : data.sources.length
                    ? 'Begin the first reading'
                    : 'Continue with a clean page'}
              </button>
            </>
          )}
          {(err || job?.error) && <p role="alert">{err || job.error}</p>}
        </>
      ) : (
        <>
          <Stroke points={data.mark} className="personal-mark" />
          <h1>
            {reading?.tested
              ? 'Your first reading.'
              : 'Your notebook is ready.'}
          </h1>
          {reading?.tested ? (
            <>
              <p className="lede">
                Sumi hid{' '}
                {reading.tested === 1 ? '1 reply' : reading.tested + ' replies'}{' '}
                before making its {reading.tested === 1 ? 'guess' : 'guesses'}.
              </p>
              <div className="pair">
                <div>
                  <strong>
                    {Math.round((100 * reading.cold) / reading.tested)}%
                  </strong>
                  <small>Guessing cold</small>
                </div>
                <div>
                  <strong>
                    {Math.round((100 * reading.warm) / reading.tested)}%
                  </strong>
                  <small>With earlier conversations</small>
                </div>
              </div>
              {reading.examples.map((v: Row) => (
                <details key={v.id}>
                  <summary>{v.question.slice(0, 85)}</summary>
                  <p>Guessed · {v.guess}</p>
                  <p>You said · {v.actual}</p>
                </details>
              ))}
            </>
          ) : (
            <p className="lede">
              {data.sources.length
                ? 'Your reading has no scorable blind predictions yet. The Mirror will show its sample size as evidence arrives.'
                : 'Your first sheet is waiting. You can bring in past conversations from the Seal whenever you like.'}
            </p>
          )}
          {reading?.pending && (
            <p className="small">
              Sumi will continue this reading when your Mac is idle and plugged
              in.
            </p>
          )}
          {reading?.issues?.length > 0 && (
            <p className="small">{reading?.issues?.[0]}</p>
          )}
          {!reading?.tested && (
            <div className="seed">
              {!choice ? (
                <button onClick={async () => quiz(await call('nextChoice'))}>
                  Try ten quick choices
                </button>
              ) : choice.done ? (
                <p>Your choices are waiting in the Scroll.</p>
              ) : (
                <>
                  <small>
                    Choice {choice.index} / {choice.total}
                  </small>
                  <h3>{choice.question}</h3>
                  <div className="inline">
                    {choice.options.map((o: string) => (
                      <button
                        disabled={busy}
                        key={o}
                        onClick={async () => {
                          setBusy(true);
                          try {
                            result(
                              await act('seedAnswer', {
                                id: choice.id,
                                actual: o,
                              }),
                            );
                            quiz(await call('nextChoice'));
                          } finally {
                            setBusy(false);
                          }
                        }}
                      >
                        {o}
                      </button>
                    ))}
                  </div>
                </>
              )}
              {last?.guess && (
                <p className="small">
                  For the previous choice, Sumi guessed “{last.guess}”.
                </p>
              )}
            </div>
          )}
          {data.proposals?.length > 0 && (
            <section className="first-proposals">
              <h2>Does this sound like you?</h2>
              {data.proposals.slice(0, 3).map((r: Row) => (
                <div className="row" key={r.id}>
                  <span>{r.text}</span>
                  <button onClick={() => act('stamp', {id: r.id})}>
                    Stamp
                  </button>
                </div>
              ))}
            </section>
          )}
          {data.detectedAgents?.length > 0 && (
            <details>
              <summary>Connect an agent on this Mac</summary>
              <ConnectAgents
                data={data}
                act={act}
                names={data.detectedAgents}
              />
            </details>
          )}
          <button className="primary" onClick={finish}>
            Open the Desk
          </button>
        </>
      )}
    </main>
  );
}
