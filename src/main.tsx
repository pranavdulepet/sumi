import React, {useEffect, useLayoutEffect, useRef, useState} from 'react';
import {createRoot} from 'react-dom/client';
import {listen} from '@tauri-apps/api/event';
import '@fontsource/shippori-mincho/latin-400.css';
import '@fontsource/zen-kaku-gothic-new/latin-400.css';
import '@fontsource/zen-kaku-gothic-new/latin-500.css';
import {Desk} from './Desk';
import {Scroll} from './Scroll';
import {Mirror} from './Mirror';
import {Seal} from './Seal';
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
    [focusWaiting, setFocusWaiting] = useState(false),
    [error, fail] = useState('');
  const queue = useRef(Promise.resolve()),
    serial = useRef(0);
  async function refresh() {
    const n = ++serial.current;
    const v = await call('state');
    if (n === serial.current) update(v);
  }
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
      if (op === 'disconnect' || op === 'connect' || op === 'surprise')
        await refresh().catch(() => {});
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
      } else if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') {
        e.preventDefault();
        menu(v => !v);
      } else if (
        (e.metaKey || e.ctrlKey) &&
        '1234'.includes(e.key) &&
        e.key.length === 1
      ) {
        e.preventDefault();
        navigate(places[Number(e.key) - 1]);
      } else if (e.key === 'Escape') menu(false);
    };
    window.addEventListener('keydown', f);
    return () => window.removeEventListener('keydown', f);
  }, [data?.setup]);
  return (
    <>
      <header data-tauri-drag-region>
        <span className="brand">墨</span>
        <span>Sumi</span>
      </header>
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
        <Onboarding data={data} act={act} finish={() => act('finish')} />
      ) : (
        <>
          <div key={place} className="unroll">
            {place === 'Desk' ? (
              <Desk
                key={data.current}
                data={data}
                act={act}
                openSeal={() => navigate('Seal', true)}
                record={record}
                onSave={(id, text) =>
                  record('save', {id, text, day: localDay()})
                }
              />
            ) : place === 'Scroll' ? (
              <Scroll data={data} act={act} />
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
                {places.map((p, i) => (
                  <button
                    key={p}
                    className={'p' + i + (place === p ? ' chosen' : '')}
                    onClick={() => navigate(p)}
                  >
                    {p}
                    <small>⌘{i + 1}</small>
                  </button>
                ))}
                <span aria-hidden="true">墨</span>
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
