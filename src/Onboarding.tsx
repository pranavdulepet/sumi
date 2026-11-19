import {ExportArrival} from './ExportArrival';
import {ConnectAgents} from './Seal';
import {Providers} from './Providers';
import {useState, useEffect} from 'react';
import {call, type Row, type Action} from './api';
import {MarkPad, Stroke, Enso} from './Ink';
export function Models({model, act}: {model: Row | null; act: Action}) {
  const [fit, fitting] = useState<Row | null>(null);
  const [download, setDownload] = useState<Row | null>(null);
  const [items, set] = useState<Row[] | null>(null),
    [working, busy] = useState(''),
    [notice, note] = useState('');
  async function refresh() {
    try {
      set(await call('models'));
    } catch (e) {
      note(String(e));
    }
  }
  useEffect(() => {
    void refresh();
    call('fit')
      .then(fitting)
      .catch(e => note(String(e)));
  }, []);
  useEffect(() => {
    let live = true;
    let timer: number | undefined;
    async function check() {
      try {
        const progress = await call('modelDownloadStatus');
        if (live) setDownload(progress);
      } catch (error) {
        if (live) note(String(error));
      }
      if (live && working === 'gguf') timer = window.setTimeout(check, 800);
    }
    void check();
    return () => {
      live = false;
      window.clearTimeout(timer);
    };
  }, [working]);
  const installed = items ?? [];
  const recommended =
    installed.find(
      m => m.name === model?.name && m.runtime === model?.runtime,
    ) ??
    installed.find(
      m =>
        m.name === 'llama3.2:3b' &&
        (!fit?.memory || m.bytes * 2 + 4e9 < fit.memory),
    ) ??
    installed.find(
      m => m.bytes && (!fit?.memory || m.bytes * 2 + 4e9 < fit.memory),
    );
  function modelRow(m: Row) {
    return (
      <button
        className="row"
        key={m.runtime + m.name}
        disabled={!!working}
        onClick={async () => {
          busy(m.name);
          try {
            await act('model', m);
            note('Ready for your notebook.');
          } catch (e) {
            note(String(e));
          } finally {
            busy('');
          }
        }}
      >
        <span>
          {m.name}
          <small>{m.runtime} · on this Mac</small>
        </span>
        <span>
          {model?.name === m.name && model?.runtime === m.runtime
            ? '✓'
            : working === m.name
              ? 'Verifying…'
              : 'Choose'}
        </span>
      </button>
    );
  }
  const enoughSpace =
    !!fit &&
    !!download &&
    (download.cached || fit.freeDisk >= download.required);
  const downloadButton = download && fit && (
    <div>
      <button
        disabled={!!working || !enoughSpace}
        onClick={async () => {
          busy('gguf');
          note('');
          try {
            await act('downloadBundled');
            note('Ready for your notebook.');
          } catch (e) {
            note(String(e));
          } finally {
            busy('');
          }
        }}
      >
        {working === 'gguf'
          ? 'Preparing Qwen on this Mac…'
          : !enoughSpace
            ? 'More disk space needed'
            : download.cached
              ? 'Set up Qwen 2.5 · up to 1.12 GB'
              : download.bytes
                ? 'Resume Qwen 2.5 download'
                : 'Download Qwen 2.5 · 1.12 GB'}
      </button>
      {working === 'gguf' && !download.cached && (
        <div className="model-download">
          <progress
            aria-label="Qwen download"
            value={download.bytes}
            max={download.total}
          />
          <small>
            {download.bytes === download.total
              ? 'Verifying model weights…'
              : (download.bytes / 1e9).toFixed(2) +
                ' of ' +
                (download.total / 1e9).toFixed(2) +
                ' GB'}
          </small>
        </div>
      )}
      {!working && !!download.bytes && !download.cached && (
        <small>
          {(download.bytes / 1e6).toFixed(0)} MB already on this Mac.
        </small>
      )}
      {!enoughSpace && (
        <small>
          Free {((download.required - fit.freeDisk) / 1e9).toFixed(2)} GB more
          on the notebook’s volume.
        </small>
      )}
    </div>
  );
  return (
    <section>
      <div className="rows">{recommended && modelRow(recommended)}</div>
      {installed.some(m => m !== recommended) && (
        <details>
          <summary>Other installed models</summary>
          <div className="rows">
            {installed.filter(m => m !== recommended).map(modelRow)}
          </div>
        </details>
      )}
      {items && !recommended && !model && downloadButton}
      {model?.runtime === 'Sumi' && (
        <p className="small">✓ {model.name} · on this Mac</p>
      )}
      <details>
        <summary>Other ways to think locally</summary>
        {items &&
          (!!recommended || model) &&
          model?.runtime !== 'Sumi' &&
          downloadButton}
        <div className="inline">
          <button
            disabled={!!working}
            onClick={async () => {
              busy('file');
              try {
                await act('importModel');
              } catch (e) {
                note(String(e));
              } finally {
                busy('');
              }
            }}
          >
            Choose GGUF file
          </button>
          <button
            disabled={!!working}
            onClick={async () => {
              busy('ollama');
              try {
                await call('startRuntime');
                await new Promise(r => setTimeout(r, 1000));
                await refresh();
              } catch (e) {
                note(String(e));
              } finally {
                busy('');
              }
            }}
          >
            Start Ollama
          </button>
          <button disabled={!!working} onClick={refresh}>
            Refresh models
          </button>
          {fit?.recommendation &&
            !installed.some(m => m.name === fit.recommendation.name) && (
              <button
                disabled={!!working}
                onClick={async () => {
                  busy('pull');
                  try {
                    await call('startRuntime');
                    await new Promise(r => setTimeout(r, 1000));
                    await act('pull', {name: fit.recommendation.name});
                    await refresh();
                  } catch (e) {
                    note(String(e));
                  } finally {
                    busy('');
                  }
                }}
              >
                {working === 'pull'
                  ? 'Downloading…'
                  : `Download ${fit.recommendation.name} with Ollama · ${(fit.recommendation.bytes / 1e9).toFixed(1)} GB`}
              </button>
            )}
        </div>
      </details>
      {notice && (
        <p role="status" className="small">
          {notice}
        </p>
      )}
    </section>
  );
}
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
  return (
    <main className={'welcome stage-' + step}>
      <div className="folio">
        SUMI <span>／</span> FIRST READING
      </div>
      {step === 0 ? (
        <>
          <h1>
            A mark of your own<span className="punct">.</span>
          </h1>
          <p className="lede">
            One brushstroke becomes the seal you leave
            <br />
            on the things Sumi gets right.
          </p>
          <MarkPad
            save={async p => {
              await act('mark', {points: p});
              go(1);
            }}
          />
        </>
      ) : step === 1 ? (
        <>
          <h1>A mind for Sumi.</h1>
          <p className="lede">Choose a model that thinks on your Mac.</p>
          <Models model={data.model} act={act} />
          <Providers data={data} act={act} />
          <button className="primary" onClick={() => go(2)}>
            {data.model ? 'Continue' : 'Use the notebook without a model'}
          </button>
        </>
      ) : step === 2 ? (
        <>
          <h1>
            {running || busy
              ? 'Reading your words.'
              : 'Your words, by invitation.'}
          </h1>
          <p className="lede">
            {running || busy
              ? 'You can close this window. If you quit Sumi, the reading resumes when you return.'
              : 'Choose the conversations Sumi may learn from.'}
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
              <Sources
                sources={data.sources}
                request={data.requestedExport}
                act={act}
              />
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
