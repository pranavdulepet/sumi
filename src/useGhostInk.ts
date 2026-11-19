import {useEffect, useLayoutEffect, useRef, useState} from 'react';
import {call, type Action, type Row} from './api';

export function useGhostInk(
  text: string,
  sheet: string,
  sealed: boolean,
  record: Action,
  hosted: boolean,
) {
  const field = useRef<HTMLTextAreaElement>(null);
  const [ready, setReady] = useState(false);
  const [candidate, show] = useState<Row | null>(null);
  const displayed = useRef<Row | null>(null);
  const composing = useRef(false);
  const correction = useRef<{
    id: string;
    prefix: string;
    actual: string;
  } | null>(null);
  const timer = useRef<number | undefined>(undefined);
  const requests = useRef(Promise.resolve());
  const suppressed = useRef<string | null>(null);
  const [revision, retry] = useState(0);
  const atEnd = () => {
    const el = field.current;
    return (
      !!el &&
      document.activeElement === el &&
      !document.hidden &&
      !composing.current &&
      el.selectionStart === el.value.length &&
      el.selectionEnd === el.value.length
    );
  };
  function hide() {
    displayed.current = null;
    show(null);
  }
  function position() {
    const enabled = atEnd();
    setReady(enabled);
    if (!enabled) hide();
  }
  function finishCorrection() {
    window.clearTimeout(timer.current);
    const c = correction.current;
    correction.current = null;
    if (c?.actual.trim())
      void record('ghostEvent', {
        id: c.id,
        status: 'edited',
        actual: c.actual,
      }).catch(() => {});
  }
  function edit(next: string) {
    const g = displayed.current;
    if (
      g &&
      !composing.current &&
      next.startsWith(text) &&
      next.length > text.length
    ) {
      correction.current = {id: g.id, prefix: text, actual: ''};
    }
    hide();
    suppressed.current = null;
    const c = correction.current;
    if (c) {
      window.clearTimeout(timer.current);
      if (!next.startsWith(c.prefix)) correction.current = null;
      else {
        c.actual = next.slice(c.prefix.length);
        if (!composing.current)
          timer.current = window.setTimeout(() => {
            finishCorrection();
            retry(n => n + 1);
          }, 1500);
      }
    }
  }
  function accept() {
    const g = displayed.current;
    if (!g || !atEnd() || field.current?.value !== g.prefix) return null;
    void record('ghostEvent', {id: g.id, status: 'accepted'}).catch(() => {});
    hide();
    return text + (/\s$/u.test(text) ? '' : ' ') + g.guess;
  }
  function dismiss() {
    suppressed.current = text;
    if (displayed.current)
      void record('ghostEvent', {
        id: displayed.current.id,
        status: 'dismissed',
      }).catch(() => {});
    hide();
  }
  useLayoutEffect(() => {
    if (
      !candidate ||
      sealed ||
      !atEnd() ||
      candidate.prefix !== field.current?.value ||
      candidate.id === displayed.current?.id
    )
      return;
    displayed.current = candidate;
    const {prefix, ...evidence} = candidate;
    void record('ghostShown', {sheet, candidate: evidence}).catch(() => {});
  }, [candidate, sealed, sheet, record]);
  useEffect(() => {
    if (
      !ready ||
      sealed ||
      !text.trim() ||
      suppressed.current === text ||
      correction.current
    )
      return;
    let live = true;
    const timeout = setTimeout(
      () => {
        // Coalesce edits made while an earlier suggestion is being written.
        requests.current = requests.current
          .then(async () => {
            if (!live || !atEnd()) return;
            const c = await call('ghost', {text});
            if (
              live &&
              c &&
              atEnd() &&
              field.current?.value === text &&
              suppressed.current !== text
            )
              show({...c, prefix: text});
          })
          .catch(() => {});
      },
      hosted ? 900 : 180,
    );
    return () => {
      live = false;
      clearTimeout(timeout);
    };
  }, [text, sheet, ready, sealed, revision, hosted]);
  useEffect(() => {
    if (sealed) hide();
  }, [sealed]);
  useEffect(() => {
    position();
    document.addEventListener('selectionchange', position);
    document.addEventListener('visibilitychange', position);
    return () => {
      document.removeEventListener('selectionchange', position);
      document.removeEventListener('visibilitychange', position);
      finishCorrection();
    };
  }, []);
  return {
    field,
    ghost:
      (ready && !sealed && candidate?.prefix === text && candidate?.guess) ||
      '',
    accept,
    dismiss,
    edit,
    events: {
      onSelect: position,
      onFocus: position,
      onBlur: () => {
        finishCorrection();
        setReady(false);
        hide();
      },
      onCompositionStart: () => {
        finishCorrection();
        composing.current = true;
        position();
      },
      onCompositionEnd: () => {
        composing.current = false;
        position();
      },
    },
  };
}
