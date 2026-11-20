import {Fragment} from 'react';

// Render the text forms a notebook uses. All model output stays in React text nodes.
function inline(text: string) {
  return text
    .split(/(\*\*[^*\n]+\*\*|`[^`\n]+`|\*[^*\n]+\*)/g)
    .map((part, i) => {
      if (part.startsWith('**') && part.endsWith('**'))
        return <strong key={i}>{part.slice(2, -2)}</strong>;
      if (part.startsWith('`') && part.endsWith('`'))
        return <code key={i}>{part.slice(1, -1)}</code>;
      if (part.startsWith('*') && part.endsWith('*'))
        return <em key={i}>{part.slice(1, -1)}</em>;
      return <Fragment key={i}>{part}</Fragment>;
    });
}
export function Reply({text}: {text: string}) {
  const blocks = text.split(/(```[^\n]*\n[\s\S]*?(?:```|$))/g);
  return (
    <div className="reply">
      {blocks.map((block, i) => {
        if (block.startsWith('```'))
          return (
            <pre key={i}>
              <code>
                {block.replace(/^```[^\n]*\n/, '').replace(/```$/, '')}
              </code>
            </pre>
          );
        return block
          .trim()
          .split(/\n\s*\n/)
          .filter(Boolean)
          .map((paragraph, j) => {
            const lines = paragraph.split('\n');
            const key = i + '-' + j;
            if (lines.every(line => /^\s*[-*] /.test(line)))
              return (
                <ul key={key}>
                  {lines.map((line, k) => (
                    <li key={k}>{inline(line.replace(/^\s*[-*] /, ''))}</li>
                  ))}
                </ul>
              );
            if (/^#{1,6} /.test(paragraph) && lines.length === 1)
              return <h3 key={key}>{inline(paragraph.replace(/^#+ /, ''))}</h3>;
            return <p key={key}>{inline(paragraph)}</p>;
          });
      })}
    </div>
  );
}
