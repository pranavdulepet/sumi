// Only accept JSON data. A malformed visual remains readable as ordinary text.
export function chart(block: string) {
  if (!block.startsWith('```chart\n')) return null;
  try {
    const value = JSON.parse(block.slice(9).replace(/```$/, ''));
    if (
      typeof value.title !== 'string' ||
      typeof value.unit !== 'string' ||
      !Array.isArray(value.values) ||
      value.values.length < 1 ||
      value.values.length > 24
    )
      return null;
    const rows: [string, number][] = value.values;
    if (
      !rows.every(
        row =>
          Array.isArray(row) &&
          row.length === 2 &&
          typeof row[0] === 'string' &&
          typeof row[1] === 'number' &&
          Number.isFinite(row[1]) &&
          row[1] >= 0,
      )
    )
      return null;
    const max = Math.max(...rows.map(r => r[1]), 1);
    return (
      <figure className="ink-chart">
        <figcaption>{value.title}</figcaption>
        <dl>
          {rows.map(([name, n], i) => (
            <div key={i}>
              <dt>{name}</dt>
              <dd>
                <i aria-hidden="true" style={{width: (100 * n) / max + '%'}} />
                {n} {value.unit}
              </dd>
            </div>
          ))}
        </dl>
      </figure>
    );
  } catch {
    return null;
  }
}
