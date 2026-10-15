import { unified } from 'unified';
import remarkParse from 'remark-parse';
import remarkMdx from 'remark-mdx';
import remarkGfm from 'remark-gfm';
import remarkStringify from 'remark-stringify';
import GithubSlugger from 'github-slugger';
import { parse as parseYaml } from 'yaml';

const parser = unified().use(remarkParse).use(remarkMdx).use(remarkGfm);
const writer = unified().use(remarkGfm).use(remarkStringify, { fences: true, bullet: '-' });
export const availabilityLabels = {
  available: 'Available', alpha: 'Alpha', experimental: 'Experimental', 'coming-soon': 'Coming soon',
};

export function walkNodes(node, visitor) {
  visitor(node);
  for (const child of node.children ?? []) walkNodes(child, visitor);
}

export function nodeText(node) {
  return node.value ?? (node.children ?? []).map(nodeText).join('');
}

export function parseDocument(source) {
  const frontmatter = source.match(/^---\r?\n([\s\S]*?)\r?\n---(?:\r?\n|$)/);
  const metadata = frontmatter ? parseYaml(frontmatter[1], { uniqueKeys: true, maxAliasCount: 50 }) : {};
  if (!metadata || typeof metadata !== 'object' || Array.isArray(metadata)) throw new Error('Frontmatter must be a YAML mapping.');
  const body = source.slice(frontmatter?.[0].length ?? 0);
  const tree = parser.parse(body);
  const anchors = new Set();
  const duplicates = [];
  const references = [];
  const codeBlocks = [];
  const slugger = new GithubSlugger();
  function anchor(id) {
    if (anchors.has(id)) duplicates.push(id);
    anchors.add(id);
  }
  walkNodes(tree, (node) => {
    if (node.type === 'heading') anchor(slugger.slug(nodeText(node)));
    if (node.type === 'link' || node.type === 'image' || node.type === 'definition') {
      references.push({ url: node.url, image: node.type === 'image', line: node.position?.start.line });
    }
    if (node.type === 'code') codeBlocks.push(node);
    if (node.type === 'mdxJsxFlowElement' || node.type === 'mdxJsxTextElement') {
      for (const attribute of node.attributes ?? []) {
        if (typeof attribute.value !== 'string') continue;
        if (attribute.name === 'id') anchor(attribute.value);
        if (attribute.name === 'href' || attribute.name === 'src') {
          references.push({ url: attribute.value, image: attribute.name === 'src', line: node.position?.start.line });
        }
      }
    }
  });
  return { metadata, body, tree, anchors, duplicateAnchors: duplicates, references, codeBlocks, hasFrontmatter: Boolean(frontmatter) };
}

function attribute(node, name) {
  return node.attributes?.find((item) => item.name === name)?.value;
}
function text(value) { return { type: 'text', value }; }
function paragraph(children) { return { type: 'paragraph', children }; }

const diagramDescriptions = {
  contract: 'Existing backend code or Acore source produces a versioned .axiom contract. Inspect, diff, and test the contract before consuming it from an existing client or Acore UI.',
  pipeline: 'Author or extract source, evaluate and validate it, build a canonical contract artifact, review and test the artifact, then distribute it to a compatible consumer.',
  inspector: 'Inspector connects application contracts, dependency relationships, security authority, and recorded runtime evidence. Questions and impact analysis read that evidence.',
  evidence: 'Keep declared source, compiled artifacts, observed execution, and qualification evidence distinct. A declaration or successful build does not prove that an application ran.',
};

function readerNodes(nodes) {
  return nodes.flatMap((node) => {
    if (node.type === 'mdxjsEsm' || node.type === 'mdxFlowExpression' || node.type === 'mdxTextExpression') return [];
    if (node.type === 'mdxJsxFlowElement' || node.type === 'mdxJsxTextElement') {
      const children = readerNodes(node.children ?? []);
      if (node.name === 'Card') {
        const title = attribute(node, 'title');
        const href = attribute(node, 'href');
        const description = attribute(node, 'description');
        if (typeof title !== 'string' || typeof href !== 'string') throw new Error('Reader cards require literal titles and links.');
        return [paragraph([{ type: 'link', url: href, children: [text(title)] }, ...(typeof description === 'string' ? [text(`: ${description}`)] : [])])];
      }
      if (node.name === 'StatusNote' || node.name === 'Callout') {
        const label = availabilityLabels[attribute(node, 'status')] ?? 'Note';
        const title = attribute(node, 'title');
        return [{ type: 'blockquote', children: [paragraph([{ type: 'strong', children: [text(`${label}${typeof title === 'string' ? `: ${title}` : ''}`)] }]), ...children] }];
      }
      if (node.name === 'StatusBadge') {
        return [text(availabilityLabels[attribute(node, 'status')] ?? 'See support and availability')];
      }
      if (node.name === 'Tab') {
        return [{ type: 'heading', depth: 3, children: [text(String(attribute(node, 'value') ?? 'Alternative'))] }, ...children];
      }
      if (node.name === 'ConceptDiagram') {
        const description = diagramDescriptions[attribute(node, 'kind')];
        if (!description) throw new Error('Concept diagrams require a reader description.');
        return [paragraph([text(description)])];
      }
      if (node.name === 'Mermaid') {
        return [paragraph([text(String(attribute(node, 'caption') ?? 'Diagram'))]), { type: 'code', lang: 'mermaid', value: String(attribute(node, 'source') ?? '') }];
      }
      if (node.name === 'CapabilityTable') throw new Error('Author capability tables as Markdown so every reader receives their contents.');
      if (node.name === 'summary') return [paragraph([{ type: 'strong', children: children.flatMap((child) => child.children ?? [child]) }])];
      if (node.name === 'span' && typeof attribute(node, 'id') === 'string') {
        return [{ type: 'html', value: `<a id="${attribute(node, 'id')}"></a>` }, ...children];
      }
      return children;
    }
    if (node.children) return [{ ...node, children: readerNodes(node.children) }];
    return [node];
  });
}

export function toReaderMarkdown(source, { canonical, title, description } = {}) {
  const document = parseDocument(source);
  const body = writer.stringify({ type: 'root', children: readerNodes(document.tree.children) });
  return [
    `# ${title ?? document.metadata.title}`,
    '',
    `Source: ${canonical}`,
    `Description: ${description ?? document.metadata.description}`,
    `Page type: ${document.metadata.pageType ?? 'guide'}`,
    ...(document.metadata.testedWith ? [`Tested with: ${document.metadata.testedWith}`] : []),
    '',
    body,
  ].join('\n');
}
