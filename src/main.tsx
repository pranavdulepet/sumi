import React, {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from 'react';
import {createRoot} from 'react-dom/client';
import {listen} from '@tauri-apps/api/event';
import '@fontsource/shippori-mincho/latin-400.css';
import {Desk} from './Desk';
import {FirstCorrection} from './FirstCorrection';
import {Scroll} from './Scroll';
import {Mirror} from './Mirror';
import {Seal} from './Seal';
import {ThinkingSettings} from './Thinking';
import {Onboarding} from './Onboarding';
import {Enso} from './Ink';
import {call, places, type Action, type Row, type Place} from './api';
import './style.css';
function localDay() {
  const d = new Date();
  return [
    d.getFullYear(),
    String(d.getMonth() + 1).padStart(2, '0'),
    String(d.getDate()).padStart(2, '0'),
  ].join('-');
}
function Notebook() {
  const [data, update] = useState<Row | null>(null),
    [place, go] = useState<Place>('Desk'),
    [nav, menu] = useState(false),
    [settings, showSettings] = useState(false),
    [teaching, teach] = useState(false),
    [replyToCorrect, selectReply] = useState<Row | null>(null),
    [focusWaiting, setFocusWaiting] = useState(false),
    [error, fail] = useState('');
  const queue = useRef(Promise.resolve()),
    serial = useRef(0);
  const refresh = useCallback(async () => {
    const n = ++serial.current;
    const v = await call('state');
    if (n === serial.current) update(v);
  }, []);
  const act: Action = async (op, args = {}) => {
    try {
      await queue.current;
      const result = await call(
        op,
        op === 'openSheet' || op === 'newSheet'
          ? {...args, day: localDay()}
          : args,
      );
      await refresh();
      return result;
    } catch (e) {
      if (
        op === 'disconnect' ||
        op === 'connect' ||
        op === 'forgetProvider' ||
        op === 'surprise'
      )
        await refresh().catch(() => {});
      if (
        op !== 'connectPlan' &&
        !op.endsWith('Correction') &&
        !(data?.firstCorrection && ['breakSeal', 'verdict'].includes(op))
      )
        fail(String(e));
      throw e;
    }
  };
  function record(op: string, args: Row = {}) {
    const p = queue.current.catch(() => {}).then(() => call(op, args));
    queue.current = p;
    return p;
  }
  function navigate(p: Place, waiting = false) {
    showSettings(false);
    setFocusWaiting(waiting);
    go(p);
    menu(false);
    void queue.current.then(refresh).catch(e => fail(String(e)));
  }
  useEffect(() => {
    let live = true;
    async function enter(shortcut = false) {
      if (!live || (document.hidden && !shortcut)) return;
      try {
        const hasDraft = Array.from(
          document.querySelectorAll<HTMLInputElement | HTMLTextAreaElement>(
            'form input:not([type=checkbox]), form textarea',
          ),
        ).some(el => el.getClientRects().length > 0 && el.value.trim());
        if (hasDraft && !shortcut) {
          await refresh();
          return;
        }
        const result = await record('enterDesk', {day: localDay(), shortcut});
        if (!live) return;
        if (shortcut || result?.newDay) {
          showSettings(false);
          go('Desk');
          menu(false);
        }
        await refresh();
      } catch (e) {
        if (live) fail(String(e));
      }
    }
    const foreground = () => {
      void enter();
    };
    foreground();
    window.addEventListener('focus', foreground);
    document.addEventListener('visibilitychange', foreground);
    const changed = listen('notebook-changed', () => {
      if (live && !document.hidden) void refresh().catch(e => fail(String(e)));
    }).catch(() => () => {});
    const off = listen('fresh-sheet', () => enter(true)).catch(() => () => {});
    return () => {
      live = false;
      window.removeEventListener('focus', foreground);
      document.removeEventListener('visibilitychange', foreground);
      void off.then(f => f());
      void changed.then(f => f());
    };
  }, []);
  useLayoutEffect(() => {
    if (!(place === 'Seal' && focusWaiting)) window.scrollTo(0, 0);
  }, [place, focusWaiting]);
  useEffect(() => {
    const f = (e: KeyboardEvent) => {
      if (!data?.setup) return;
      if (
        e.altKey &&
        !e.metaKey &&
        !e.ctrlKey &&
        !e.shiftKey &&
        e.code === 'Space'
      ) {
        e.preventDefault();
        if (!e.repeat && !e.isComposing)
          void act('enterDesk', {day: localDay(), shortcut: true})
            .then(() => navigate('Desk'))
            .catch(() => {});
      } else if ((e.metaKey || e.ctrlKey) && e.key === ',') {
        e.preventDefault();
        menu(false);
        showSettings(true);
      } else if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') {
        e.preventDefault();
        navigate('Scroll');
      } else if (
        (e.metaKey || e.ctrlKey) &&
        '1234'.includes(e.key) &&
        e.key.length === 1
      ) {
        e.preventDefault();
        navigate(places[Number(e.key) - 1]);
      } else if (e.key === 'Escape') {
        menu(false);
        showSettings(false);
      }
    };
    window.addEventListener('keydown', f);
    return () => window.removeEventListener('keydown', f);
  }, [data?.setup]);
  useEffect(() => {
    const pts: number[][] = data?.mark || [];
    const d = pts
      .map(
        ([x, y], i) =>
          (i ? 'L' : 'M') + (x * 100).toFixed(1) + ' ' + (y * 100).toFixed(1),
      )
      .join(' ');
    const ink =
      '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><path fill="none" stroke="white" stroke-width="6" stroke-linecap="round" d="' +
      d +
      '"/></svg>';
    document.documentElement.style.setProperty(
      '--signature',
      'url("data:image/svg+xml,' + encodeURIComponent(ink) + '")',
    );
  }, [JSON.stringify(data?.mark)]);
  const lessonOpen = data?.setup && (teaching || !!data.firstCorrection);
  return (
    <>
      <header data-tauri-drag-region aria-hidden="true" />
      {error && (
        <div className="error" role="alert">
          {error}{' '}
          <button
            onClick={() => {
              fail('');
              if (!data) void refresh().catch(e => fail(String(e)));
            }}
          >
            {data ? 'Dismiss' : 'Retry'}
          </button>
        </div>
      )}
      {!data ? (
        <div className="startup">
          <Enso size={46} />
        </div>
      ) : !data.setup ? (
        <Onboarding
          data={data}
          act={act}
          refresh={refresh}
          finish={() => act('finish')}
        />
      ) : lessonOpen ? (
        <main className="first-lesson">
          <FirstCorrection
            reply={replyToCorrect}
            data={data}
            act={async (op, args) => {
              const value = await act(op, args);
              if (op === 'finish') {
                teach(false);
                selectReply(null);
                navigate('Desk');
              }
              return value;
            }}
          />
        </main>
      ) : (
        <>
          <div
            aria-hidden="true"
            className="landscape"
            style={
              {
                '--reveal':
                  Math.min(
                    85,
                    32 +
                      6 *
                        Math.sqrt(
                          (data.notes?.length || 0) +
                            data.entries.length +
                            (data.skills?.length || 0),
                        ),
                  ) + '%',
              } as React.CSSProperties
            }
          />
          <div key={place} className="unroll">
            {settings ? (
              <ThinkingSettings
                data={data}
                act={act}
                close={() => showSettings(false)}
                teach={() => teach(true)}
                agents={() => navigate('Seal')}
                readings={() => navigate('Mirror')}
              />
            ) : place === 'Desk' ? (
              <Desk
                key={data.current}
                data={data}
                act={act}
                teach={() => teach(true)}
                correctReply={turn => {
                  selectReply({...turn, sheet: data.current});
                  teach(true);
                }}
                openSeal={() => navigate('Seal', true)}
                record={record}
                onSave={(id, text) =>
                  record('save', {id, text, day: localDay()})
                }
              />
            ) : place === 'Scroll' ? (
              <Scroll
                data={data}
                act={act}
                readings={() => navigate('Mirror')}
                teach={() => teach(true)}
              />
            ) : place === 'Mirror' ? (
              <Mirror data={data} act={act} />
            ) : (
              <Seal data={data} act={act} focusWaiting={focusWaiting} />
            )}
          </div>
          {nav && (
            <div className="ring-back" onClick={() => menu(false)}>
              <nav
                aria-label="Places"
                className="ring"
                onClick={e => e.stopPropagation()}
              >
                {places.slice(0, 2).map((p, i) => (
                  <button
                    key={p}
                    className={'p' + i + (place === p ? ' chosen' : '')}
                    onClick={() => navigate(p)}
                  >
                    {i === 0 ? 'Conversation' : 'What Sumi knows'}
                    <small>{i === 0 ? '⌘1' : '⌘K'}</small>
                  </button>
                ))}
                <button
                  className="ring-settings"
                  onClick={() => {
                    menu(false);
                    showSettings(true);
                  }}
                >
                  Settings<small>⌘,</small>
                </button>
              </nav>
            </div>
          )}
          <button
            className="places"
            aria-expanded={nav}
            onClick={() => menu(!nav)}
            aria-label="Places"
          >
            <span>Places</span>
            <Enso size={34} />
          </button>
        </>
      )}
    </>
  );
}
createRoot(document.getElementById('root')!).render(<Notebook />);
