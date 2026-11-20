import {useEffect, useState} from 'react';
import {call, type Row, type Action} from './api';
export function SheetImage({
  turn,
  sheet,
  act,
}: {
  turn: Row;
  sheet: string;
  act: Action;
}) {
  const [src, set] = useState(''),
    [pending, wait] = useState(false),
    [error, fail] = useState('');
  useEffect(() => {
    let live = true;
    if (turn.image)
      void call('sheetImage', {id: turn.image})
        .then(v => {
          if (live) set(v);
        })
        .catch(e => {
          if (live) fail(String(e));
        });
    return () => {
      live = false;
    };
  }, [turn.image]);
  if (!turn.image && !turn.imagePrompt) return null;
  return (
    <figure className="sheet-image">
      {src && <img src={src} alt={turn.question} />}
      {turn.imagePrompt && (
        <>
          <blockquote>{turn.imagePrompt}</blockquote>
          <small>
            OpenRouter receives this prompt and may charge your account.
          </small>
          <button
            disabled={pending}
            onClick={async () => {
              wait(true);
              try {
                await act('renderImage', {
                  sheet,
                  turn: turn.id,
                  prompt: turn.imagePrompt,
                });
              } catch (e) {
                fail(String(e));
              } finally {
                wait(false);
              }
            }}
          >
            {pending ? 'Drawing…' : 'Generate this image'}
          </button>
        </>
      )}
      {error && <small role="alert">{error}</small>}
    </figure>
  );
}
