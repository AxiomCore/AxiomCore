import { renderDocumentationDiagram } from '@/lib/mermaid.mjs';

export function Mermaid({ source, caption }: { source: string; caption: string }) {
  const { svg, width, height } = renderDocumentationDiagram(source);
  return (
    <figure className="docs-diagram">
      <div className="docs-diagram-scroll" tabIndex={0} role="region" aria-label={caption}>
        <img
          src={`data:image/svg+xml,${encodeURIComponent(svg)}`}
          alt={caption}
          width={width}
          height={height}
        />
      </div>
      <figcaption>{caption}</figcaption>
      <details>
        <summary>Diagram source</summary>
        <pre><code className="language-mermaid">{source}</code></pre>
      </details>
    </figure>
  );
}
