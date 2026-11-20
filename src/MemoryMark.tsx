import {useRef, useState} from 'react';
import {day, type Action, type Row} from './api';

export function MemoryMark({
  note,
  current,
  act,
}: {
  note: Row;
  current?: Row;
  act: Action;
}) {
  const [open, setOpen] = useState(false);
  const [working, setWorking] = useState(false);
  const container = useRef<HTMLSpanElement>(null);
  const until = current?.validUntil;
  const ended = until && new Date(until).getTime() <= Date.now();
  return (
    <span
      className="memory-mark"
      ref={container}
      onMouseEnter={() => setOpen(true)}
      onMouseLeave={() => setOpen(false)}
      onBlur={e => {
        if (!container.current?.contains(e.relatedTarget)) setOpen(false);
      }}
      onKeyDown={e => {
        if (e.key === 'Escape') setOpen(false);
      }}
    >
      <button
        type="button"
        aria-label="Show remembered preference"
        aria-expanded={open}
        onFocus={() => setOpen(true)}
        onClick={() => setOpen(true)}
      >
        ●
      </button>
      {open && (
        <span
          className="memory-slip"
          role="group"
          aria-label="Remembered words"
        >
          <span>{note.text}</span>
          <small>
            Since {day(note.validFrom)} {new Date(note.validFrom).getFullYear()}
            {until ? (ended ? ' · ended ' : ' · through ') + day(until) : ''}
          </small>
          {current && !ended && (
            <button
              type="button"
              disabled={working}
              onClick={async () => {
                setWorking(true);
                try {
                  await act(
                    note.kind === 'entry' ? 'retireEntry' : 'closeNote',
                    {id: note.id, text: note.text},
                  );
                } finally {
                  setWorking(false);
                }
              }}
            >
              Not true anymore
            </button>
          )}
        </span>
      )}
    </span>
  );
}
