import type {Row} from './api';

type Reading = {
  month: Date;
  model: string;
  grader: string;
  total: number;
  cold: number;
  personal: number;
};

function monthlyReadings(scores: Row[]): Reading[] {
  const groups = new Map<string, Reading>();
  const validGrade = (v: unknown) =>
    typeof v === 'number' && Number.isInteger(v) && v >= 0 && v <= 3;
  for (const score of scores) {
    if (
      score.scope !== 'held-out' ||
      !validGrade(score.grade) ||
      !validGrade(score.coldGrade)
    )
      continue;
    const when = new Date(score.predictedAt || score.date);
    if (!Number.isFinite(when.getTime())) continue;
    const month = new Date(when.getFullYear(), when.getMonth(), 1);
    const model = score.model || 'Unrecorded model';
    const grader = score.grader || 'Unrecorded grader';
    const key = JSON.stringify([month.getTime(), model, grader]);
    const group = groups.get(key) ?? {
      month,
      model,
      grader,
      total: 0,
      cold: 0,
      personal: 0,
    };
    group.total++;
    if (score.grade >= 2) group.personal++;
    if (score.coldGrade >= 2) group.cold++;
    groups.set(key, group);
  }
  return [...groups.values()].sort(
    (a, b) =>
      b.month.getTime() - a.month.getTime() ||
      a.model.localeCompare(b.model) ||
      a.grader.localeCompare(b.grader),
  );
}

export function BlindReadings({scores}: {scores: Row[]}) {
  const groups = monthlyReadings(scores);
  return (
    <details className="blind-readings">
      <summary>Does personalization help?</summary>
      {groups.length ? (
        <>
          <p className="small">
            Each hidden reply gets two guesses from the same model. The
            difference shows how often earlier context helped, in percentage
            points.
          </p>
          <table>
            <caption className="sr">
              Matched guesses by prediction month and model
            </caption>
            <thead>
              <tr>
                <th scope="col">Reading</th>
                <th scope="col">Cold</th>
                <th scope="col">Personal</th>
                <th scope="col">Difference</th>
              </tr>
            </thead>
            <tbody>
              {groups.map(r => {
                const diff = (100 * (r.personal - r.cold)) / r.total;
                return (
                  <tr
                    key={JSON.stringify([r.month.getTime(), r.model, r.grader])}
                  >
                    <th scope="row">
                      {r.month.toLocaleString(undefined, {
                        month: 'short',
                        year: 'numeric',
                      })}
                      <small>{r.model}</small>
                      {r.grader !== r.model && (
                        <small>
                          {r.grader === 'Unrecorded grader'
                            ? 'Grader unrecorded'
                            : `Graded by ${r.grader}`}
                        </small>
                      )}
                    </th>
                    <td>
                      {r.cold}/{r.total}
                    </td>
                    <td>
                      {r.personal}/{r.total}
                    </td>
                    <td>
                      {diff.toLocaleString(undefined, {
                        maximumFractionDigits: 1,
                        signDisplay: 'exceptZero',
                      })}{' '}
                      pp
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </>
      ) : (
        <p className="small">
          After a blind reading, you’ll see how often Sumi matched the same
          replies with and without earlier context.
        </p>
      )}
    </details>
  );
}
