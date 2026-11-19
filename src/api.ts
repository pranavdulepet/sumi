import {invoke} from '@tauri-apps/api/core';
export type Row = Record<string, any>;
export type Action = (name: string, args?: Row) => Promise<any>;
export const call: Action = (operation, args = {}) =>
  invoke('notebook', {operation, args});
export const places = ['Desk', 'Scroll', 'Mirror', 'Seal'] as const;
export type Place = (typeof places)[number];
export const sections: Record<string, string> = {
  deciding: 'How you decide',
  writing: 'How you write',
  people: 'People',
  words: 'Your words',
  goals: 'What you are working toward',
  facts: 'Facts',
};
export function day(date: string) {
  return new Date(date).toLocaleDateString(undefined, {
    month: 'short',
    day: 'numeric',
  });
}
