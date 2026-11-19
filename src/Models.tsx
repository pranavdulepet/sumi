import {useState, useEffect} from 'react';
import {call, type Row, type Action} from './api';
export function Models({
  model,
  act,
  simple = false,
}: {
  model: Row | null;
  act: Action;
  simple?: boolean;
}) {
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
      {!simple && installed.some(m => m !== recommended) && (
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
      {!simple && (
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
      )}
      {notice && (
        <p role="status" className="small">
          {notice}
        </p>
      )}
    </section>
  );
}
