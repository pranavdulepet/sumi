import type {Row} from './api';

const common = new Set(
  'a about after again all also am an and any are as at be because been before being but by can could did do does doing each for from get goal had has have how i if in into is it its keep like make me more most my need of on one only or our out should so some than that the their them then there these they this those through time to toward try up us want was we were what when which while will with work would you your'.split(
    ' ',
  ),
);

function terms(text: string) {
  const words = text
    .normalize('NFKC')
    .toLowerCase()
    .match(/[\p{L}\p{N}]+/gu);
  return new Set((words ?? []).filter(w => w.length >= 3 && !common.has(w)));
}

export function paragraphAt(text: string, caret: number) {
  const start = text.slice(0, caret).lastIndexOf('\n') + 1;
  const end = text.indexOf('\n', caret);
  return text.slice(start, end < 0 ? text.length : end);
}

export function marginGoal(paragraph: string, entries: Row[], muted?: Row) {
  const now = Date.now();
  const context = terms(paragraph);
  if (context.size < 2 || Date.parse(muted?.quietUntil) > now) return;

  let best: Row | undefined;
  let bestScore = 0;
  for (const entry of entries) {
    if (
      entry.section !== 'goals' ||
      !(entry.authored === true || entry.status === 'stamped') ||
      Date.parse(muted?.[entry.id]?.until) > now
    )
      continue;
    const strength = entry.currentConfidence ?? entry.confidence ?? 0.5;
    if (strength < 0.35) continue;
    const goal = terms(entry.text);
    const matches = [...goal].filter(word => context.has(word)).length;
    if (matches < 2) continue;
    const score =
      matches + matches / goal.size + strength * (entry.importance ?? 0.5);
    if (score > bestScore) {
      bestScore = score;
      best = entry;
    }
  }
  return best;
}
