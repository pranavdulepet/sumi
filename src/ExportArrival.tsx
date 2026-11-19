import {useState} from 'react';
import {type Action, type Row} from './api';

export function ExportArrival({
  request,
  act,
  desk = false,
}: {
  request?: Row;
  act: Action;
  desk?: boolean;
}) {
  const [added, setAdded] = useState('');
  const [working, setWorking] = useState(false);
  const offers: Row[] = request?.offers || [];
  if (!request || (desk && !offers.length)) {
    return desk && added ? (
      <p className="small export-name" role="status">
        Added {added} to your sources.
      </p>
    ) : null;
  }
  async function accept(v: Row) {
    setWorking(true);
    try {
      await act('acceptExport', {path: v.path});
      setAdded(v.name);
    } finally {
      setWorking(false);
    }
  }
  return (
    <div>
      <p className="small" role="status">
        {offers.length
          ? offers.length === 1
            ? 'A chat export is ready in Downloads.'
            : 'Chat exports are ready in Downloads.'
          : request.issue || 'Waiting for your chat export in Downloads.'}
      </p>
      {offers.map(v => (
        <button
          className="row export-name"
          key={v.path}
          disabled={working}
          onClick={() => void accept(v).catch(() => {})}
        >
          Add {v.name} to Sumi
        </button>
      ))}
      <button
        className="quiet small"
        disabled={working}
        onClick={() => {
          setAdded('');
          void act('cancelExport').catch(() => {});
        }}
      >
        {offers.length ? 'Dismiss' : 'Stop looking'}
      </button>
    </div>
  );
}
