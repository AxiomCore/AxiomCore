// Docs search for Ask AI. The index is built once at build time from the same
// Markdown the site publishes for machine readers (`/docs/<page>.mdx`), and is
// small and precomputed so a lookup in a Cloudflare Pages Function is cheap:
// parse once per isolate, then score only the terms in the question (BM25).
import GithubSlugger from 'github-slugger';

/**
 * @typedef {{ u: string, t: string, d: string }} IndexPage
 * @typedef {{ p: number, a: string, h: string }} IndexChunk
 * @typedef {{ version: number, pages: IndexPage[], chunks: IndexChunk[], lengths: number[], average: number, terms: Record<string, string> }} DocsIndex
 * @typedef {{ url: string, title: string, description?: string, markdown: string }} CorpusPage
 */

/** Where the static export publishes the index. */
export const ASK_AI_INDEX_PATH = '/ask-ai/docs-index.json';

const STOP_WORDS = new Set(
  'a an and are as at be but by can do does for from has have how i if in into is it its me my no not of on or our so than that the their them then there these they this to us use used using was we what when where which who why will with you your'.split(' '),
);
const K1 = 1.2;
const B = 0.75;
/** Terms that appear in more than this share of sections carry no signal. */
const MAX_DOCUMENT_FREQUENCY = 0.4;

/** Lowercase word-like tokens; keeps names such as `atmx-web`, `0.149.1`, `axiom_flutter`. */
export function tokenize(text) {
  const tokens = [];
  for (const raw of String(text).toLowerCase().match(/[a-z0-9][a-z0-9._-]*/g) ?? []) {
    const token = raw.replace(/[._-]+$/, '');
    if (token.length < 2 || STOP_WORDS.has(token)) continue;
    tokens.push(token);
    // Also index the parts of compound names, so `atmx-web` matches `web`.
    if (/[._-]/.test(token)) {
      for (const part of token.split(/[._-]+/)) if (part.length > 1 && !STOP_WORDS.has(part)) tokens.push(part);
    }
  }
  return tokens;
}

/**
 * Split one page's reader Markdown into heading sections. The anchors match the
 * site's heading ids, which are GitHub-style slugs.
 */
export function splitSections(markdown) {
  const sections = [];
  const slugger = new GithubSlugger();
  let current = { heading: '', anchor: '', lines: [] };
  let fence = false;
  for (const line of String(markdown).split('\n')) {
    if (/^\s*(```|~~~)/.test(line)) fence = !fence;
    const match = !fence && /^(#{2,3})\s+(.+?)\s*#*\s*$/.exec(line);
    if (match) {
      sections.push(current);
      const heading = match[2].replace(/[`*_]/g, '').trim();
      current = { heading, anchor: slugger.slug(heading), lines: [] };
      continue;
    }
    current.lines.push(line);
  }
  sections.push(current);
  return sections
    .map(({ heading, anchor, lines }) => ({ heading, anchor, text: lines.join('\n').trim() }))
    .filter((section) => section.text || section.heading);
}

/** Header lines that `toReaderMarkdown` adds before the page body. */
function readerBody(markdown) {
  return String(markdown).replace(/^# .*\n\nSource: .*\nDescription: .*\nPage type: .*\n(?:Tested with: .*\n)?/, '');
}

/**
 * @param {CorpusPage[]} pages
 * @returns {DocsIndex}
 */
export function buildDocsIndex(pages) {
  /** @type {DocsIndex} */
  const index = { version: 1, pages: [], chunks: [], lengths: [], average: 0, terms: {} };
  /** @type {Map<string, string[]>} */
  const postings = new Map();
  for (const page of pages) {
    const pageId = index.pages.length;
    index.pages.push({ u: page.url, t: page.title, d: page.description ?? '' });
    for (const section of splitSections(readerBody(page.markdown))) {
      const chunkId = index.chunks.length;
      index.chunks.push({ p: pageId, a: section.anchor, h: section.heading });
      const counts = new Map();
      // Titles and headings say what a section is about, so they weigh more.
      const weighted = [
        ...tokenize(page.title), ...tokenize(page.title),
        ...tokenize(section.heading), ...tokenize(section.heading), ...tokenize(section.heading),
        ...tokenize(section.text),
      ];
      for (const token of weighted) counts.set(token, (counts.get(token) ?? 0) + 1);
      index.lengths.push(weighted.length);
      for (const [token, count] of counts) {
        if (!postings.has(token)) postings.set(token, []);
        postings.get(token).push(`${chunkId.toString(36)}:${Math.min(count, 35).toString(36)}`);
      }
    }
  }
  const total = index.chunks.length;
  index.average = total ? index.lengths.reduce((sum, value) => sum + value, 0) / total : 0;
  for (const [token, list] of [...postings].sort(([a], [b]) => a.localeCompare(b))) {
    if (list.length / total > MAX_DOCUMENT_FREQUENCY) continue;
    index.terms[token] = list.join(' ');
  }
  return index;
}

/**
 * Best-matching sections.
 * @param {DocsIndex} index
 * @param {string} query
 * @returns {{ url: string, page: string, title: string, heading: string, anchor: string, score: number }[]}
 */
export function searchDocsIndex(index, query, limit = 8) {
  const total = index.chunks.length;
  const scores = new Map();
  for (const token of new Set(tokenize(query))) {
    const list = index.terms[token];
    if (!list) continue;
    const entries = list.split(' ');
    const idf = Math.log(1 + (total - entries.length + 0.5) / (entries.length + 0.5));
    for (const entry of entries) {
      const [id36, tf36] = entry.split(':');
      const id = parseInt(id36, 36);
      const tf = parseInt(tf36, 36);
      const norm = tf + K1 * (1 - B + (B * index.lengths[id]) / index.average);
      scores.set(id, (scores.get(id) ?? 0) + (idf * tf * (K1 + 1)) / norm);
    }
  }
  return [...scores]
    .sort((a, b) => b[1] - a[1])
    .slice(0, limit)
    .map(([id, score]) => {
      const chunk = index.chunks[id];
      const page = index.pages[chunk.p];
      return {
        url: chunk.a ? `${page.u}#${chunk.a}` : page.u,
        page: page.u,
        title: page.t,
        heading: chunk.h || page.t,
        anchor: chunk.a,
        score: Math.round(score * 100) / 100,
      };
    });
}

/**
 * The published reader file for a page route, or null if it is not a docs page.
 * @param {DocsIndex} index
 * @param {string} url
 * @returns {string | null}
 */
export function readerPathFor(index, url) {
  let path;
  try {
    path = new URL(url, 'https://docs.axiomcore.dev').pathname.replace(/\/+$/, '') || '/';
  } catch {
    return null;
  }
  if (!index.pages.some((page) => page.u === path)) return null;
  return path === '/' ? '/docs/index.mdx' : `/docs${path}.mdx`;
}

/**
 * The text of one section of a page's reader Markdown.
 * @param {string} markdown
 * @param {string | undefined} anchor
 */
export function sectionText(markdown, anchor) {
  const sections = splitSections(readerBody(markdown));
  const section = sections.find((candidate) => candidate.anchor === (anchor ?? '')) ?? sections[0];
  return section ? section.text : '';
}
