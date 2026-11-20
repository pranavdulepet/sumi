import {useState} from 'react';
import {type Action, type Row, call} from './api';
import {Page} from './Page';
import {Sources} from './Onboarding';
import {Models} from './Models';
import {Stroke} from './Ink';
import {NotebookData} from './NotebookData';
import {Providers} from './Providers';

function PlanButton({
  act,
  connected,
  onConnected,
}: {
  act: Action;
  connected: boolean;
  onConnected?: () => void;
}) {
  const [busy, wait] = useState(false);
  const [error, fail] = useState('');
  return (
    <>
      <button
        className="primary"
        disabled={busy}
        onClick={async () => {
          wait(true);
          fail('');
          try {
            await act('connectPlan');
            onConnected?.();
          } catch (e) {
            fail(String(e));
          } finally {
            wait(false);
          }
        }}
      >
        {busy
          ? 'Waiting for ChatGPT…'
          : connected
            ? 'Reconnect ChatGPT'
            : 'Continue with ChatGPT'}
      </button>
      {busy && (
        <p role="status">
          Finish signing in with OpenAI. Sumi will check that your plan can
          answer.
        </p>
      )}
      {error && <p role="alert">{error}</p>}
    </>
  );
}

export function Think({
  act,
  data,
  next,
}: {
  act: Action;
  data: Row;
  next: () => void;
}) {
  const [local, chooseLocal] = useState(false);
  const [busy, wait] = useState(false);
  const [ready, confirm] = useState('');
  const [error, fail] = useState('');
  const [signing, sign] = useState(false);
  const connect: Action = async (op, args) => {
    sign(true);
    try {
      return await act(op, args);
    } finally {
      sign(false);
    }
  };
  async function onMac() {
    wait(true);
    fail('');
    try {
      const [models, fit]: [Row[], Row] = await Promise.all([
        call('models'),
        call('fit'),
      ]);
      const installed = models.filter(
        m => !m.bytes || m.bytes * 2 + 4e9 < fit.memory,
      );
      const choice =
        installed.find(m => m.name === fit.recommendation?.name) ??
        installed.find(m => m.name === 'llama3.2:3b') ??
        installed[0];
      if (choice) {
        await act('model', choice);
        confirm('Ready to think on this Mac.');
      } else {
        chooseLocal(true);
      }
    } catch (e) {
      fail(String(e));
    } finally {
      wait(false);
    }
  }
  if (ready)
    return (
      <>
        <p role="status" className="lede">
          {ready}
        </p>
        <button className="primary" onClick={next}>
          Continue
        </button>
      </>
    );
  if (local)
    return (
      <>
        <h1>Think on this Mac.</h1>
        <Models
          simple
          model={data.model}
          act={async (op, args) => {
            const r = await act(op, args);
            if (
              ['downloadBundled', 'model', 'importModel'].includes(op) &&
              r?.name
            )
              confirm('Ready to think on this Mac.');
            return r;
          }}
        />
        <button onClick={() => chooseLocal(false)}>Use ChatGPT instead</button>
      </>
    );
  return (
    <section className="think-page" aria-label="How Sumi thinks">
      <Stroke className="think-mark" points={data.mark || []} />
      <div className="think-copy">
        <h1>
          Your notebook stays on this Mac.
          <br />
          Sumi thinks with your ChatGPT plan.
        </h1>
        <p className="small">
          OpenAI receives the passages Sumi uses. You can change this in
          Settings.
        </p>
      </div>
      <div className="think-actions">
        {!busy && (
          <PlanButton
            act={connect}
            connected={false}
            onConnected={() => confirm('Connected to your ChatGPT plan.')}
          />
        )}
        <button disabled={busy || signing} onClick={onMac}>
          {busy ? 'Checking this Mac…' : 'Think on this Mac instead'}
        </button>
      </div>
      {error && <p role="alert">{error}</p>}
    </section>
  );
}

export function ThinkingSettings({
  data,
  act,
  close,
  teach,
  agents,
  readings,
}: {
  data: Row;
  act: Action;
  close: () => void;
  agents: () => void;
  readings: () => void;
  teach: () => void;
}) {
  const [pendingSearch, setPendingSearch] = useState<boolean | null>(null);
  const plan = data.model?.runtime === 'ChatGPT';
  const [busy, working] = useState(false);
  return (
    <Page name="Settings" margin={<button onClick={close}>Done</button>}>
      <section>
        <h2>How Sumi thinks</h2>
        {data.model && <button onClick={teach}>Start with a correction</button>}
        <p>
          {plan
            ? 'Your notebook stays on this Mac. Sumi thinks with your ChatGPT plan.'
            : data.model
              ? 'Sumi thinks on this Mac with ' + data.model.name + '.'
              : 'Choose how Sumi thinks.'}
        </p>
        {!plan && (
          <p className="small">
            With ChatGPT, OpenAI receives passages used for predictions,
            suggestions, and readings.
          </p>
        )}
        <PlanButton act={act} connected={plan} />
        {data.providers.some((p: Row) => p.id === 'ChatGPT') && (
          <button
            onClick={() =>
              void act('forgetProvider', {provider: 'ChatGPT'}).catch(() => {})
            }
          >
            Disconnect ChatGPT
          </button>
        )}
        {plan && data.providers.find((p: Row) => p.primary)?.email && (
          <small>{data.providers.find((p: Row) => p.primary).email}</small>
        )}
        {(plan || data.planUsage > 0) && (
          <>
            <p className="small">
              {data.planUsage || 0} of 1,200 ChatGPT requests this month.
            </p>
            <button
              onClick={() => void act('planUsageSettings').catch(() => {})}
            >
              Manage plan usage in ChatGPT ↗
            </button>
          </>
        )}
      </section>
      <details>
        <summary>Think on this Mac</summary>
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
      <details>
        <summary>Web search</summary>
        <label className="check">
          <input
            type="checkbox"
            checked={pendingSearch ?? !!data.autoListings}
            disabled={pendingSearch !== null}
            onChange={async e => {
              const value = e.target.checked;
              setPendingSearch(value);
              try {
                await act('autoListings', {value});
              } finally {
                setPendingSearch(null);
              }
            }}
          />
          Automatically look up weekend listings
        </label>
        <p className="small">
          Exa receives the city, dates and a broad activity category. Other
          searches show you the query first. Private sheets stay offline.
        </p>
      </details>
      <button onClick={agents}>Connected agents</button>
      <button onClick={readings}>Review predictions</button>
      <Providers data={data} act={act} />
      <NotebookData root={data.root}/>
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
    </Page>
  );
}
