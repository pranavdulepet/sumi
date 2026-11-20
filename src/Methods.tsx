import {useState} from 'react';
import type {Row, Action} from './api';

export function MethodEditor({
  skill,
  act,
  close,
}: {
  skill: Row;
  act: Action;
  close: () => void;
}) {
  const [title, name] = useState<string>(skill.title);
  const [body, write] = useState<string>(skill.body);
  const [busy, wait] = useState(false);
  const [error, fail] = useState('');
  return (
    <form
      className="method-editor"
      onSubmit={async e => {
        e.preventDefault();
        if (busy) return;
        wait(true);
        try {
          await act('saveSkill', {...skill, title, body});
          close();
        } catch (err) {
          fail(String(err));
        } finally {
          wait(false);
        }
      }}
    >
      <label>
        Name this skill
        <input
          autoFocus
          required
          maxLength={100}
          value={title}
          onChange={e => name(e.target.value)}
        />
      </label>
      <label>
        Steps to keep
        <textarea
          required
          rows={5}
          maxLength={3000}
          value={body}
          onChange={e => write(e.target.value)}
        />
      </label>
      {skill.private && <small>This skill runs on your Mac.</small>}
      <div className="inline">
        <button
          className="primary"
          disabled={busy || !title.trim() || !body.trim()}
        >
          Keep this skill
        </button>
        <button type="button" onClick={close}>
          Cancel
        </button>
      </div>
      {error && <p role="alert">{error}</p>}
    </form>
  );
}
export function Methods({skills, act}: {skills: Row[]; act: Action}) {
  const [editing, edit] = useState(''),
    [removing, remove] = useState('');
  if (!skills?.length) return null;
  return (
    <section>
      <h2>Skills you keep</h2>
      {skills.map(skill => (
        <details key={skill.id}>
          <summary>{skill.title}</summary>
          {editing === skill.id ? (
            <MethodEditor skill={skill} act={act} close={() => edit('')} />
          ) : (
            <>
              <p className="handwritten">{skill.body}</p>
              <div className="inline">
                <button onClick={() => edit(skill.id)}>Edit steps</button>
                <button
                  onClick={() =>
                    removing === skill.id
                      ? void act('removeSkill', {id: skill.id}).catch(() => {})
                      : remove(skill.id)
                  }
                >
                  {removing === skill.id ? 'Confirm removal' : 'Remove skill'}
                </button>
                {removing === skill.id && (
                  <button onClick={() => remove('')}>Cancel</button>
                )}
              </div>
            </>
          )}
        </details>
      ))}
    </section>
  );
}
