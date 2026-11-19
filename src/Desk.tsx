import {ExportArrival} from './ExportArrival';
import {useEffect, useMemo, useRef, useState} from 'react';
import {type Row, type Action, day} from './api';
import {useGhostInk} from './useGhostInk';
import {Stroke} from './Ink';
import {marginGoal, paragraphAt} from './margin';
export function Desk({
  data,
  act,
  openSeal,
  onSave,
  record,
}: {
  data: Row;
  act: Action;
  openSeal: () => void;
  record: Action;
  onSave: (id: string, text: string) => Promise<void>;
}) {
  const [text, setText] = useState<string>(data.sheet),
    [error, setError] = useState(''),
    [saved, setSaved] = useState(''),
    [actual, setActual] = useState(''),
    [busy, setBusy] = useState(false),
    [caret, setCaret] = useState(0),
    [composing, setComposing] = useState(false),
    [settled, setSettled] = useState(''),
    [archive, showArchive] = useState(false),
    [breakOpen, open] = useState(false);
  const rev = useRef(0);
  const sealed = data.sealed;
  const ink = useGhostInk(
    text,
    data.current,
    !!sealed,
    record,
    data.model?.runtime === 'ChatGPT',
  );
  const {field, ghost} = ink;
  const waiting: Row[] = data.log.filter((v: Row) => v.status === 'waiting');
  const names = [...new Set(waiting.map(v => v.name).filter(Boolean))];
  const from =
    names.length === 1
      ? names[0]
      : names.length === 2
        ? names.join(' and ')
        : names.length
          ? names.length + ' agents'
          : '';
  const verdict = data.scores.find((r: Row) => r.id === sealed?.id);
  function change(v: string) {
    const seq = ++rev.current;
    setText(v);
    setSaved('Saving…');
    void onSave(data.current, v)
      .then(() => {
        if (seq === rev.current) setSaved('Saved');
      })
      .catch(e => setError(String(e)));
  }
  useEffect(() => {
    const el = field.current;
    if (el) {
      el.style.height = 'auto';
      el.style.height = Math.max(150, el.scrollHeight) + 'px';
    }
  }, [text]);
  const decision =
    /\b(should i|should we|whether|decide|do i|do we|or should)\b/i.test(text);
  useEffect(() => {
    if (!decision || !data.model || sealed || !text.trim().endsWith('?'))
      return;
    let live = true;
    const timer = setTimeout(() => {
      if (live)
        void act('sealGuess', {id: data.current}).catch(e =>
          setError(String(e)),
        );
    }, 2100);
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [text, decision, data.current, sealed?.id, data.model?.name]);
  const paragraph = paragraphAt(text, caret);
  useEffect(() => {
    if (!paragraph.trim() || composing || data.marginOff) {
      setSettled('');
      return;
    }
    const pause = setTimeout(() => setSettled(paragraph), 700);
    return () => clearTimeout(pause);
  }, [paragraph, composing, data.marginOff]);
  const candidate = useMemo(
    () => marginGoal(settled, data.entries, data.marginMuted),
    [settled, data.entries, data.marginMuted],
  );
  const margin =
    !data.marginOff &&
    !composing &&
    candidate &&
    (paragraph === settled ||
      marginGoal(paragraph, [candidate], data.marginMuted))
      ? candidate
      : undefined;
  return (
    <main className="page desk">
      <div className="folio">
        {new Date().toLocaleDateString(undefined, {
          weekday: 'long',
          day: 'numeric',
          month: 'long',
        })}
      </div>
      <div className="heading-line">
        <h1>The Desk</h1>
        <button className="quiet small" onClick={() => showArchive(!archive)}>
          {archive ? 'Close sheets' : 'Sheets'}{' '}
          <span aria-hidden="true">⌄</span>
        </button>
      </div>
      {archive && (
        <div className="archive rows">
          <button onClick={() => act('newSheet')}>＋ New sheet</button>
          {data.sheets.map((s: Row) => (
            <button
              key={s.id}
              className="row"
              onClick={() =>
                act('openSheet', {id: s.id}).then(() => showArchive(false))
              }
            >
              <span>{s.title}</span>
              <small>{day(s.date)}</small>
            </button>
          ))}
        </div>
      )}
      <ExportArrival request={data.requestedExport} act={act} desk />
      {waiting.length > 0 && (
        <button className="waiting" onClick={openSeal}>
          {waiting.length} {waiting.length === 1 ? 'question' : 'questions'}
          {from && ' from ' + from} {waiting.length === 1 ? 'is' : 'are'}{' '}
          waiting for you ↗
        </button>
      )}
      <div className="writing">
        {ghost && (
          <div className="ghost-layer" aria-hidden="true">
            <span>{text}</span>
            {/\s$/u.test(text) ? '' : ' '}
            {ghost}
          </div>
        )}
        <textarea
          aria-label="Desk sheet"
          autoFocus
          placeholder="Begin here…"
          ref={field}
          spellCheck
          value={text}
          {...ink.events}
          onSelect={e => {
            ink.events.onSelect();
            setCaret(e.currentTarget.selectionStart);
          }}
          onCompositionStart={() => {
            setComposing(true);
            ink.events.onCompositionStart();
          }}
          onCompositionEnd={() => {
            setComposing(false);
            ink.events.onCompositionEnd();
          }}
          onChange={e => {
            setCaret(e.target.selectionStart);
            ink.edit(e.target.value);
            change(e.target.value);
          }}
          onKeyDown={e => {
            if (e.nativeEvent.isComposing) return;
            if (
              e.key === 'Tab' &&
              !e.shiftKey &&
              !e.altKey &&
              !e.ctrlKey &&
              !e.metaKey
            ) {
              const next = ink.accept();
              if (next !== null) {
                e.preventDefault();
                change(next);
              }
            } else if (e.key === 'Escape') ink.dismiss();
          }}
        />
      </div>
      {ghost && (
        <p className="small" aria-live="polite">
          <kbd>Tab</kbd> accepts the light ink
          <span className="sr">: {ghost}</span>
        </p>
      )}
      {sealed ? (
        <div className="seal-block">
          <div className="inline">
            <button
              className="seal-button"
              aria-label="Open sealed guess"
              onClick={() => open(!breakOpen)}
            >
              <Stroke points={data.mark} />
            </button>
            <span>
              {sealed.actual ? 'Opened guess' : 'Sealed guess'}
              <small>
                {sealed.actual
                  ? 'Your decision is recorded.'
                  : 'Open after you decide.'}
              </small>
            </span>
          </div>
          {(breakOpen || sealed.actual) &&
            (!sealed.actual ? (
              <form
                onSubmit={async e => {
                  e.preventDefault();
                  await act('breakSeal', {id: sealed.id, actual});
                  setActual('');
                }}
              >
                <label>
                  Your decision
                  <input
                    required
                    value={actual}
                    onChange={e => setActual(e.target.value)}
                    autoFocus
                  />
                </label>
                <button className="primary">Break the seal</button>
              </form>
            ) : (
              <>
                <p className="quote">{sealed.guess}</p>
                <p className="muted">You decided · {sealed.actual}</p>
                {verdict ? (
                  <p className="small">
                    {verdict.grade >= 2 ? 'A match' : 'A miss'} · in the Mirror
                  </p>
                ) : (
                  <div className="inline">
                    <button
                      onClick={() =>
                        act('verdict', {id: sealed.id, right: true})
                      }
                    >
                      That was right
                    </button>
                    <button
                      onClick={() =>
                        act('verdict', {id: sealed.id, right: false})
                      }
                    >
                      It missed
                    </button>
                  </div>
                )}
              </>
            ))}
        </div>
      ) : (
        decision && (
          <button
            className="text-action"
            disabled={busy}
            onClick={async () => {
              setBusy(true);
              try {
                await act('sealGuess', {id: data.current});
              } catch (e) {
                setError(String(e));
              } finally {
                setBusy(false);
              }
            }}
          >
            {busy ? 'Folding the guess…' : 'Seal a guess'}
          </button>
        )
      )}
      {margin && (
        <aside className="margin-note" aria-label="Goal from your Scroll">
          <small>
            From your Scroll{margin.updated && ' · ' + day(margin.updated)}
          </small>
          {margin.text}{' '}
          <button
            aria-label="Dismiss margin note"
            title="Quiet all margin notes for 24 hours"
            onClick={() => {
              void act('dismissMargin', {id: margin.id})
                .then(() => setSaved('Margin notes quiet for 24 hours.'))
                .catch(e => setError(String(e)));
            }}
          >
            ×
          </button>
        </aside>
      )}
      <div aria-live="polite" className="small quiet save-state">
        {error || saved}
      </div>
    </main>
  );
}
