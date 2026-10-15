import { renderMermaidSVG } from 'beautiful-mermaid';

/** Render only the local, declarative diagram subset used by the docs. */
export function renderDocumentationDiagram(source) {
  if (!/^(?:flowchart|graph|sequenceDiagram|erDiagram)\b/.test(source.trim())) {
    throw new Error('Use a flowchart, sequence diagram, or ER diagram.');
  }
  if (source.length > 12_000 || /%%\{|^\s*(?:click|style|classDef|linkStyle)\b|<\/?[a-z][^>]*>/im.test(source)) {
    throw new Error('Documentation diagrams cannot contain HTML, directives, links, or custom styles.');
  }

  // The diagram uses our bundled font. Remove the renderer's optional web-font
  // import so builds and readers never depend on a third-party font service.
  const svg = renderMermaidSVG(source, {
    bg: '#080808',
    fg: '#eeeeee',
    accent: '#ff3d00',
    muted: '#b3b3b3',
    line: '#777777',
    border: '#666666',
    font: 'Commit Mono',
    transparent: true,
    padding: 20,
  }).replace(/@import\s+url\([^;]+;/g, '');

  if (/<(?:script|foreignObject)\b|\bon\w+\s*=|(?:href|src)\s*=|@import|url\((?!#)/i.test(svg)) {
    throw new Error('Rendered diagram contains an active or external resource.');
  }
  const viewBox = svg.match(/viewBox="([^"]+)"/)?.[1].split(/\s+/).map(Number);
  if (!viewBox || viewBox.length !== 4 || viewBox.some((value) => !Number.isFinite(value))) {
    throw new Error('Rendered diagram is missing a finite viewBox.');
  }
  return { svg, width: Math.ceil(viewBox[2]), height: Math.ceil(viewBox[3]) };
}
