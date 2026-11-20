import type {ReactNode} from 'react';
export function Page({
  name,
  margin,
  children,
}: {
  name: string;
  margin?: ReactNode;
  children: ReactNode;
}) {
  return (
    <main className="page spread" aria-label={name}>
      <aside className="book-margin">
        <h1>{name}</h1>
        {margin}
      </aside>
      <div className="leaf">{children}</div>
    </main>
  );
}
