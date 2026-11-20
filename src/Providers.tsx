import {useState} from 'react';
import {type Action, type Row} from './api';
export function Providers({data, act}: {data: Row; act: Action}) {
  const prior = data.providers.find((r: Row) => r.id === 'OpenRouter');
  const [kind, pick] = useState('OpenRouter'),
    [model, setModel] = useState(prior?.model || ''),
    [imageModel, setImageModel] = useState(prior?.imageModel || ''),
    [secret, setSecret] = useState(''),
    [endpoint, setEndpoint] = useState(''),
    [cap, limit] = useState(prior?.cap || 0),
    [enabled, enable] = useState(!!prior?.enabled),
    [conversation, allowConversation] = useState(!!prior?.conversation),
    [quiz, allowQuiz] = useState(false),
    [backtest, allowBacktest] = useState(!!prior?.backtest),
    [busy, wait] = useState(false),
    [message, note] = useState('');
  function choose(k: string) {
    const old = data.providers.find((r: Row) => r.id === k);
    pick(k);
    setImageModel(old?.imageModel || '');
    setModel(old?.model || '');
    setSecret('');
    setEndpoint(old?.endpoint || '');
    limit(old?.cap || 0);
    enable(!!old?.enabled);
    allowConversation(!!old?.conversation);
    allowQuiz(!!old?.quiz);
    allowBacktest(!!old?.backtest);
    note('');
  }
  return (
    <details>
      <summary>Optional providers</summary>
      <p className="small">
        These connections can help with background readings when Sumi is
        thinking on this Mac.
      </p>
      <label>
        Provider
        <select value={kind} onChange={e => choose(e.target.value)}>
          {[
            'OpenRouter',
            'OpenAI',
            'Anthropic',
            'Gemini',
            'Compatible',
            'Jev',
          ].map(k => (
            <option key={k}>{k}</option>
          ))}
        </select>
      </label>
      {kind === 'OpenRouter' && (
        <button
          disabled={busy}
          onClick={async () => {
            wait(true);
            try {
              await act('oauth', {provider: kind});
              note('Account connected. Choose the allowance below.');
            } finally {
              wait(false);
            }
          }}
        >
          {busy ? 'Waiting for sign-in…' : 'Connect OpenRouter'} ↗
        </button>
      )}
      <label>
        API key
        <input
          type="password"
          autoComplete="off"
          placeholder={
            data.providers.some((r: Row) => r.id === kind)
              ? 'Saved in Keychain'
              : 'Stored in macOS Keychain'
          }
          value={secret}
          onChange={e => setSecret(e.target.value)}
        />
      </label>
      {kind === 'Compatible' && (
        <label>
          HTTPS API base URL
          <input
            value={endpoint}
            onChange={e => setEndpoint(e.target.value)}
            placeholder="https://provider.example/v1"
          />
        </label>
      )}
      {kind !== 'Jev' && (
        <label>
          Model ID
          <input
            value={model}
            onChange={e => setModel(e.target.value)}
            placeholder="Model available on your account"
          />
        </label>
      )}
      <label>
        Monthly request allowance
        <input
          type="number"
          value={cap}
          min="0"
          max="10000"
          onChange={e => limit(+e.target.value)}
        />
      </label>
      <label className="check">
        <input
          type="checkbox"
          checked={enabled}
          onChange={e => enable(e.target.checked)}
        />
        {kind === 'Jev'
          ? 'Allow cleaned grading decisions'
          : 'Allow idle reflection'}
      </label>
      {kind === 'OpenRouter' && (
        <label className="check">
          <input
            type="checkbox"
            checked={conversation}
            onChange={e => allowConversation(e.target.checked)}
          />
          Use this model for deeper conversations when ChatGPT is connected.
          OpenRouter receives the passages used and may charge your account.
        </label>
      )}
      {kind === 'OpenRouter' && (
        <label>
          Image model ID (optional)
          <input
            value={imageModel}
            onChange={e => setImageModel(e.target.value)}
            placeholder="Model with image output"
          />
        </label>
      )}
      {kind === 'Jev' && enabled && (
        <label className="check">
          <input
            type="checkbox"
            checked={quiz}
            onChange={e => allowQuiz(e.target.checked)}
          />
          Include my ten setup choices in Jev predictions
        </label>
      )}
      {kind !== 'Jev' && enabled && (
        <label className="check">
          <input
            type="checkbox"
            checked={backtest}
            onChange={e => allowBacktest(e.target.checked)}
          />
          Allow cleaned blind predictions and grading while idle
        </label>
      )}
      <div className="inline">
        <button
          className="primary"
          onClick={async () => {
            await act('provider', {
              provider: kind,
              model,
              key: secret,
              endpoint,
              cap,
              enabled,
              conversation,
              imageModel,
              quiz,
              backtest,
            });
            setSecret('');
            note('Preference saved.');
          }}
        >
          Save provider
        </button>
        {data.providers.some((p: Row) => p.id === kind) && (
          <button
            onClick={async () => {
              await act('forgetProvider', {provider: kind});
              setSecret('');
              enable(false);
              note('Disconnected on this Mac.');
            }}
          >
            Disconnect
          </button>
        )}
      </div>
      {message && (
        <p role="status" className="small">
          {message}
        </p>
      )}
    </details>
  );
}
