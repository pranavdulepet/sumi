import {useRef} from 'react';
import {call, type Row} from './api';

export async function readFiles(
  files: File[],
  available: number,
): Promise<Row[]> {
  if (files.length > available)
    throw Error('Choose up to four files at a time.');
  const results: Row[] = [];
  for (const file of files) {
    if (file.size > 8 * 1024 * 1024) throw Error('Choose a file under 8 MB.');
    const data = await new Promise<string>((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => resolve(String(reader.result).split(',')[1]);
      reader.onerror = () => reject(Error('This file could not be opened.'));
      reader.readAsDataURL(file);
    });
    results.push(await call('readAttachment', {name: file.name, data}));
  }
  return results;
}
export function Attachments({
  files,
  disabled,
  add,
  remove,
}: {
  files: Row[];
  disabled: boolean;
  add: (files: File[]) => void;
  remove: (index: number) => void;
}) {
  const input = useRef<HTMLInputElement>(null);
  return (
    <div className="attachments">
      <input
        ref={input}
        type="file"
        hidden
        multiple
        accept=".txt,.md,.csv,.tsv,.pdf,.doc,.docx,.rtf,.xlsx,.png,.jpg,.jpeg,.webp,.heic,.tiff"
        aria-label="Choose attachments"
        onChange={e => {
          add(Array.from(e.target.files || []));
          e.target.value = '';
        }}
      />
      <button
        disabled={disabled || files.length >= 4}
        onClick={() => input.current?.click()}
      >
        Attach a file
      </button>
      {files.map((f, i) => (
        <button
          key={i}
          disabled={disabled}
          onClick={() => remove(i)}
          aria-label={'Remove ' + f.name}
        >
          {f.name} ×
        </button>
      ))}
    </div>
  );
}
