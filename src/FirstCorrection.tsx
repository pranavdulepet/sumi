import {useState} from 'react';
import type {Action, Row} from './api';
import {Stroke} from './Ink';

const pages = {
  write: {
    title: 'What did you have to correct?',
    note: 'Think of the last time someone suggested one thing and you wanted another. What did you tell them?',
    action: 'Read my correction',
    pending: 'Reading…',
  },
  review: {
    title: 'Is this what you meant?',
    note: 'Edit the sentence until it sounds like you. Your seal keeps it in your notebook.',
    action: 'Keep this sentence',
    pending: 'Saving…',
  },
  guess: {
    title: 'Where would this come up again?',
    note: 'Give Sumi a related decision. Leave your answer for the next page.',
    action: 'Make a guess',
    pending: 'Writing the guess…',
  },
  choose: {
    title: 'What would you choose?',
    note: 'Sumi has sealed its guess. Write your own decision to open it.',
    action: 'Open the seal',
    pending: 'Opening…',
  },
  reveal: {
    title: 'How did Sumi do?',
    note: '',
    action: 'It matched',
    pending: 'Saving…',
  },
};

export function FirstCorrection({data, act}: {data: Row; act: Action}) {
  const lesson = data.firstCorrection;
  const sealed = lesson?.sealed;
  const [revising, revise] = useState(false);
  const [correction, write] = useState(lesson?.original ?? '');
  const [draft, edit] = useState<string | null>(null);
  const [question, ask] = useState(lesson?.question ?? '');
  const [actual, decide] = useState('');
  const [busy, wait] = useState(false);
  const [error, fail] = useState('');
  const score = sealed && data.scores.find((v: Row) => v.id === sealed.id);
  const learning = !lesson || revising;
  const reviewing = !learning && !lesson.kept;
  const guessing = lesson?.kept && !sealed;
  const opening = sealed && !sealed.actual;
  const rule = draft ?? lesson?.draft ?? '';
  const page = learning
    ? pages.write
    : reviewing
      ? pages.review
      : guessing
        ? pages.guess
        : opening
          ? pages.choose
          : pages.reveal;
  async function run(task: () => Promise<unknown>) {
    if (busy) return;
    wait(true);
    fail('');
    try {
      await task();
    } catch (err) {
      fail(String(err));
    } finally {
      wait(false);
    }
  }
  async function advance() {
    if (learning) {
      await act('readCorrection', {text: correction});
      edit(null);
      revise(false);
    } else if (reviewing) {
      await act('keepCorrection', {id: lesson.id, text: rule});
    } else if (guessing) {
      await act('tryCorrection', {id: lesson.id, question});
    } else if (opening) {
      await act('breakSeal', {id: sealed.id, actual});
    } else if (!score) {
      await act('verdict', {id: sealed.id, right: true});
    } else {
      await act('finish');
    }
  }
  const disabled = learning
    ? !correction.trim()
    : reviewing
      ? !rule.trim()
      : guessing
        ? question.trim().length < 8
        : opening
          ? !actual.trim()
          : false;
  return (
    <form
      className="correction-page"
      onSubmit={e => {
        e.preventDefault();
        void run(advance);
      }}
    >
      <div className="correction-writing">
        <h1>{page.title}</h1>
        {learning ? (
          <textarea
            aria-label="Your correction"
            autoFocus
            maxLength={1600}
            value={correction}
            onChange={e => write(e.target.value)}
            placeholder="I asked for … because …"
          />
        ) : reviewing ? (
          <>
            <blockquote>{lesson.original}</blockquote>
            <label>
              Sumi’s reading
              <textarea
                aria-label="What Sumi learned"
                maxLength={1600}
                value={rule}
                onChange={e => edit(e.target.value)}
              />
            </label>
          </>
        ) : guessing ? (
          <textarea
            aria-label="Your next decision"
            autoFocus
            maxLength={1600}
            value={question}
            onChange={e => ask(e.target.value)}
            placeholder="Should I …"
          />
        ) : opening ? (
          <>
            <blockquote>{lesson.question}</blockquote>
            <textarea
              aria-label="Your decision"
              autoFocus
              maxLength={1600}
              value={actual}
              onChange={e => decide(e.target.value)}
            />
          </>
        ) : (
          <>
            <p>{lesson.question}</p>
            <label>Sumi guessed</label>
            <blockquote>{sealed.guess}</blockquote>
            <label>You chose</label>
            <p>{sealed.actual}</p>
          </>
        )}
      </div>
      <aside>
        {page.note && <p>{page.note}</p>}
        {(guessing || sealed?.actual) && (
          <div className="correction-note">
            <small>
              {guessing
                ? 'Your saved sentence'
                : sealed.ruleIds?.includes(lesson.id)
                  ? 'The sentence Sumi read'
                  : 'Your sentence wasn’t used for this guess'}
            </small>
            <p>{lesson.draft}</p>
          </div>
        )}
        {score && (
          <p role="status">
            {score.grade >= 2 ? 'Recorded as a match.' : 'Recorded as a miss.'}
          </p>
        )}
        <button
          type="submit"
          className="ink-action"
          disabled={busy || disabled}
        >
          <Stroke points={data.mark} />
          <span>
            {busy ? page.pending : score ? 'Open the Desk' : page.action}
          </span>
        </button>
        {reviewing && (
          <button
            type="button"
            disabled={busy}
            onClick={() => {
              write(lesson.original);
              revise(true);
            }}
          >
            Change my correction
          </button>
        )}
        {sealed?.actual && !score && (
          <button
            type="button"
            disabled={busy}
            onClick={() =>
              void run(() => act('verdict', {id: sealed.id, right: false}))
            }
          >
            It missed
          </button>
        )}
        {data.setup && !sealed?.actual && (
          <button
            type="button"
            disabled={busy}
            onClick={() => void run(() => act('finish'))}
          >
            Return to my notebook
          </button>
        )}
        {error && <p role="alert">{error}</p>}
      </aside>
    </form>
  );
}
