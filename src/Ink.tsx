import {useRef, useState} from 'react';
export type Point = [number, number];
export function Enso({
  confidence = 0.86,
  size = 36,
}: {
  confidence?: number;
  size?: number;
}) {
  return (
    <svg
      aria-hidden="true"
      width={size}
      height={size}
      viewBox="0 0 100 100"
      fill="none"
    >
      <g
        transform="rotate(-79 50 50)"
        stroke="currentColor"
        strokeLinecap="round"
      >
        <circle
          cx="50"
          cy="50"
          r="39"
          strokeWidth="5.1"
          pathLength="100"
          strokeDasharray={`${Math.max(4, confidence * 98)} 100`}
        />
        <circle
          cx="49"
          cy="51"
          r="40"
          strokeWidth="1.5"
          pathLength="100"
          strokeDasharray={`${confidence * 92} 100`}
        />
        <path d="M90 50 C91 73 72 90 52 90" strokeWidth="2.8" />
      </g>
    </svg>
  );
}
export function Stroke({
  points,
  className = '',
}: {
  points: Point[];
  className?: string;
}) {
  const d = points
    .map(
      ([x, y], i) =>
        (i ? 'L' : 'M') +
        ' ' +
        (x * 240).toFixed(2) +
        ' ' +
        (y * 150).toFixed(2),
    )
    .join(' ');
  return (
    <svg className={className} viewBox="0 0 240 150" aria-hidden="true">
      <path
        d={d}
        fill="none"
        stroke="currentColor"
        strokeWidth="5"
        strokeLinejoin="round"
        strokeLinecap="round"
      />
      <path
        d={d}
        transform="translate(-.8 1.4)"
        fill="none"
        stroke="currentColor"
        strokeWidth="1"
        opacity=".55"
      />
    </svg>
  );
}
export function MarkPad({save}: {save: (p: Point[]) => void}) {
  const [points, set] = useState<Point[]>([]);
  const drawing = useRef(false);
  function append(e: React.PointerEvent<HTMLDivElement>, first = false) {
    const b = e.currentTarget.getBoundingClientRect();
    const p: Point = [
      Math.max(0, Math.min(1, (e.clientX - b.left) / b.width)),
      Math.max(0, Math.min(1, (e.clientY - b.top) / b.height)),
    ];
    set(prev => (first ? [p] : prev.length < 2000 ? [...prev, p] : prev));
  }
  return (
    <>
      <div
        className="markpad"
        role="img"
        aria-label="Draw your personal brushstroke with a mouse or trackpad"
        onPointerDown={e => {
          e.currentTarget.setPointerCapture(e.pointerId);
          drawing.current = true;
          append(e, true);
        }}
        onPointerMove={e => drawing.current && append(e)}
        onPointerUp={() => (drawing.current = false)}
        onPointerCancel={() => (drawing.current = false)}
      >
        <i />
        <i />
        <i />
        <i />
        {points.length ? (
          <Stroke points={points} />
        ) : (
          <span className="brush-hint">Draw one stroke</span>
        )}
      </div>
      <div className="inline centered">
        {points.length > 1 ? (
          <>
            <button className="primary" onClick={() => save(points)}>
              Keep my mark
            </button>
            <button onClick={() => set([])}>Draw again</button>
          </>
        ) : (
          <button
            className="small quiet"
            onClick={() =>
              set(
                Array.from({length: 64}, (_, i) => {
                  const a = (i / 64) * 5.7 - 1.15;
                  return [0.5 + 0.23 * Math.cos(a), 0.5 + 0.33 * Math.sin(a)];
                }),
              )
            }
          >
            Use an ensō instead
          </button>
        )}
      </div>
    </>
  );
}
