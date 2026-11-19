import {useState} from 'react';
import {type Action, type Row, call} from './api';
import {Models} from './Models';
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
      const installed = models
        .filter(m => !m.bytes || m.bytes * 1.4 + 3e9 < fit.memory)
        .sort((a, b) => b.bytes - a.bytes);
      if (installed.length) {
        await act('model', installed[0]);
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
    <>
      <h1>How Sumi thinks.</h1>
      <p className="lede">
        Your notebook stays on this Mac. Sumi thinks with your ChatGPT plan.
      </p>
      <p className="small">
        OpenAI receives the passages Sumi uses to read, predict, and finish your
        writing.
      </p>
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
    </>
  );
}

export function ThinkingSettings({
  data,
  act,
  close,
}: {
  data: Row;
  act: Action;
  close: () => void;
}) {
  const plan = data.model?.runtime === 'ChatGPT';
  return (
    <main className="page">
      <div className="heading-line">
        <h1>Settings</h1>
        <button onClick={close}>Done</button>
      </div>
      <section>
        <h2>How Sumi thinks</h2>
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
      <Providers data={data} act={act} />
    </main>
  );
}
