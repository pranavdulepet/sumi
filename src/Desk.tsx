import {ExportArrival} from './ExportArrival';
import {useEffect, useMemo, useRef, useState} from 'react';
import {type Row, type Action, day} from './api';
import {useGhostInk} from './useGhostInk';
import {Stroke} from './Ink';
import {MemoryMark} from './MemoryMark';
import {SearchResult} from './SearchResult';
import {Reply} from './Reply';
import {marginGoal, paragraphAt} from './margin';
export function Desk({
  data,
  act,
  openSeal,
  teach,
  correctReply,
  onSave,
  record,
}: {
  data: Row;
  act: Action;
  openSeal: () => void;
  teach: () => void;
  correctReply: (turn: Row) => void;
  record: Action;
  onSave: (id: string, text: string) => Promise<void>;
}) {
  const [text, setText] = useState<string>(data.sheet),
    [error, setError] = useState(''),
    [notice, setNotice] = useState(''),
    [privateSheet, setPrivate] = useState(
      data.sheets?.find((s: Row) => s.id === data.current)?.private === true,
    ),
    [actual, setActual] = useState(''),
    [busy, setBusy] = useState(false),
    [caret, setCaret] = useState(0),
    [composing, setComposing] = useState(false),
    [settled, setSettled] = useState(''),
    [archive, showArchive] = useState(false),
    [breakOpen, open] = useState(false);
  const sending = useRef(false);
  const pending = useRef<{text: string; id: string} | null>(null);
  const turns: Row[] = data.chat?.turns || [];
  const leaf = useRef<HTMLDivElement>(null);
  async function send() {
    if (!text.trim() || sending.current || busy) return;
    sending.current = true;
    setBusy(true);
    setError('');
    if (pending.current?.text !== text)
      pending.current = {text, id: crypto.randomUUID().replaceAll('-', '')};
    try {
      await act('askSheet', {
        sheet: data.current,
        turn: pending.current.id,
        text,
        private: privateSheet,
      });
      const turn = pending.current.id;
      requestAnimationFrame(() => {
        const reply = leaf.current?.querySelector<HTMLElement>(
          '[data-turn="' + turn + '"]',
        );
        if (reply) {
          reply.focus({preventScroll: true});
          reply.scrollIntoView({block: 'nearest'});
        }
      });
      setText('');
      ink.dismiss();
      setCaret(0);
      pending.current = null;
    } catch (e) {
      setError(String(e));
    } finally {
      sending.current = false;
      setBusy(false);
    }
  }
  async function switchSheet(id?: string) {
    setBusy(true);
    try {
      await act(id ? 'openSheet' : 'newSheet', id ? {id} : {});
      showArchive(false);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  }
  useEffect(() => {
    const sheets = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'o') {
        e.preventDefault();
        showArchive(v => !v);
      }
    };
    window.addEventListener('keydown', sheets);
    return () => window.removeEventListener('keydown', sheets);
  }, []);
  const sealed = data.sealed;
  const ink = useGhostInk(
    text,
    data.current,
    !!sealed ||
      (data.model?.runtime === 'ChatGPT' &&
        (privateSheet ||
          data.chat?.private ||
          data.skills?.some((s: Row) => s.private))),
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
    setText(v);
    void onSave(data.current, v).catch(e => setError(String(e)));
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
      <aside className="desk-margin">
        <time>
          {new Date().toLocaleDateString(undefined, {
            month: 'long',
            day: 'numeric',
          })}
        </time>
        {!sealed && (
          <button
            className="ink-action ask"
            disabled={busy || !text.trim()}
            onClick={send}
          >
            <Stroke points={data.mark} />
            {busy ? 'Thinking…' : 'Ask Sumi'}
          </button>
        )}
        <button
          onClick={() =>
            turns.length ? correctReply(turns[turns.length - 1]) : teach()
          }
        >
          {turns.length ? 'Correct this reply' : 'Teach a correction'}
        </button>
        <button onClick={() => showArchive(!archive)}>
          {archive ? 'Close sheets' : 'Sheets'} <small>⌘O</small>
        </button>
        {!data.chat?.networkClosed && (
          <label className="check">
            <input
              type="checkbox"
              checked={privateSheet}
              disabled={busy}
              onChange={async e => {
                const value = e.target.checked;
                setPrivate(value);
                try {
                  await act('sheetPrivacy', {id: data.current, value});
                } catch (err) {
                  setPrivate(!value);
                  setError(String(err));
                }
              }}
            />
            {data.model?.runtime === 'ChatGPT'
              ? 'Keep this sheet local'
              : 'Keep this sheet offline'}
          </label>
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
                  .then(() => setNotice('Margin notes quiet for 24 hours.'))
                  .catch(e => setError(String(e)));
              }}
            >
              ×
            </button>
          </aside>
        )}
      </aside>
      <div className="desk-leaf" ref={leaf}>
        {archive && (
          <div className="archive rows">
            <button disabled={busy} onClick={() => switchSheet()}>
              ＋ New sheet
            </button>
            {data.sheets.map((s: Row) => (
              <button
                key={s.id}
                className="row"
                disabled={busy}
                onClick={() => switchSheet(s.id)}
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
        {turns.map(t => (
          <article
            className="exchange"
            key={t.id}
            tabIndex={-1}
            aria-label="Conversation turn"
            data-turn={t.id}
          >
            <p className="asked">{t.question}</p>
            <small>Sumi</small>
            <Reply
              text={
                t.usedMemory
                  ? t.reply.replace(/^(.*?)([.!?])(?=\s|$)/, '$1$2 [[m1]]')
                  : t.reply
              }
              mark={index => {
                const note = t.usedMemory
                  ? index === 0
                    ? t.usedMemory
                    : null
                  : t.notes?.[index];
                return (
                  note && (
                    <MemoryMark
                      note={note}
                      current={(note.kind === 'entry'
                        ? data.entries
                        : data.notes
                      )?.find(
                        (n: Row) => n.id === note.id && n.text === note.text,
                      )}
                      act={act}
                    />
                  )
                );
              }}
            />
            <SearchResult turn={t} sheet={data.current} act={act} />
            {t.timeline?.length > 0 && (
              <ol
                className="memory-timeline"
                aria-label="Your earlier instructions"
              >
                {t.timeline.map((p: Row, i: number) => (
                  <li key={i}>
                    <time dateTime={p.when}>
                      {p.approximate ? 'Sheet dated ' : ''}
                      {new Date(p.when).toLocaleString(undefined, {
                        month: 'short',
                        day: 'numeric',
                        year: 'numeric',
                        hour: 'numeric',
                        minute: '2-digit',
                      })}
                    </time>
                    <p>{p.words}</p>
                  </li>
                ))}
              </ol>
            )}
            {t.worksheet && (
              <dl className="calculations" aria-label="Working">
                {t.tools
                  .filter((v: Row) => v.value != null || v.date)
                  .map((v: Row) => (
                    <div key={v.tool}>
                      <dt>{v.tool === 'Calendar' ? 'Date' : 'Calculation'}</dt>
                      <dd>{v.display}</dd>
                    </div>
                  ))}
              </dl>
            )}
            <details className="receipt">
              <summary>Behind this reply</summary>
              <p>
                {t.timeline?.length
                  ? 'Quoted from your notebook'
                  : t.runtime + ' · ' + t.model}
              </p>
              {t.selector &&
                (t.timeline?.length > 0 ||
                  t.selector !== t.model ||
                  t.selectorRuntime !== t.runtime) && (
                  <p>
                    Selected by {t.selectorRuntime} · {t.selector}
                  </p>
                )}
              {t.rules.map((r: string) => (
                <blockquote key={r}>{r}</blockquote>
              ))}
              {t.skill && <p>Skill · {t.skill}</p>}
              {(t.tools || []).map((tool: Row) => (
                <div key={tool.tool}>
                  <p>
                    {tool.tool} · {tool.display}
                  </p>
                  {tool.passages?.map((p: Row, i: number) => (
                    <blockquote key={i}>
                      <small>
                        {p.approximate ? 'Sheet dated ' : ''}
                        {new Date(p.when).toLocaleString()}
                      </small>
                      {p.words}
                    </blockquote>
                  ))}
                </div>
              ))}
            </details>
          </article>
        ))}
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
            aria-keyshortcuts="Enter Shift+Enter"
            autoFocus
            placeholder={turns.length ? 'And then…' : 'What’s on your mind?'}
            disabled={busy}
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
              if (!e.shiftKey && !e.altKey && e.key === 'Enter' && !sealed) {
                e.preventDefault();
                void send();
                return;
              }
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
                      {verdict.grade >= 2 ? 'A match' : 'A miss'} · in the
                      Mirror
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
        <div aria-live="polite" className="small quiet save-state">
          {error || notice}
        </div>
        {sealed && (
          <button
            className="text-action"
            disabled={busy}
            onClick={() => switchSheet()}
          >
            Start a conversation
          </button>
        )}
      </div>
    </main>
  );
}
