# Public documentation guardrails

These instructions apply to this directory and every file below it. Read them
before adding, moving, or updating public documentation, examples, diagrams,
screenshots, navigation, or documentation tooling.

## Preserve the product and its design

- Keep the approved theme, fonts, colors, spacing, layout, and existing component
  behavior. Content improvements do not authorize a visual redesign.
- Write for developers evaluating AxiomCore, integrating it into an existing
  stack, or building an application with Acore. Make the next step clear.
- Use **AxiomCore** for the platform, **Acore** for the language, and **Axiom CLI**
  for `axiom`. Distinguish `.acore` source, `.axiom` packages and contracts, and
  `.axiomapp` application artifacts.
- Treat existing-stack adoption and Acore authoring as separate, connected
  paths. State which frontend, backend, database, SDK, host, and artifact each
  path uses. Acore is optional at supported integration edges.

## Preserve information during a rewrite

- Inventory the affected pages and sections before restructuring them. Map
  every existing instruction, constraint, example, option, default, diagnostic,
  compatibility detail, and support qualification to its destination.
- Consolidate repeated explanations into one canonical page and link to it.
  Preserve distinct facts and exceptions. Shorter prose must not remove detail.
- Replace obsolete or incorrect guidance with verified current behavior and
  record the reason in the internal migration record. Do not preserve false
  claims merely to preserve their wording.
- Keep a section-level migration record and evidence in the internal docs
  repository. Public pages must remain usable without access to private files,
  sibling checkouts, maintainer tools, or unpublished artifacts.
- Retain established URLs where practical. For a moved page, add a direct,
  tested permanent redirect in `lib/legacy-redirects.mjs`. Preserve referenced
  heading anchors or provide a working compatibility anchor. Update and verify
  `lib/legacy-heading-anchors.json` when adding migrations. Invisible legacy
  IDs may retain historical names solely for bookmark compatibility. Update navigation,
  internal links, canonical metadata, sitemap, search, and LLM-reader outputs.

## Write clear, complete prose

- Lead with what the reader can do. Use short sentences, concrete verbs, and
  familiar words. Define a technical term when the reader first needs it.
- Do not use em dash characters in authored documentation, captions, labels,
  alt text, or newly authored example comments. Rephrase with a period, comma,
  colon, or parentheses. Choose examples without that character. If external
  output contains it, explain the result in prose rather than altering and
  presenting the output as verbatim.
- Avoid stock AI phrasing, sales copy, filler, rhetorical questions, and claims
  such as "seamless", "effortless", "zero bugs", or "works everywhere".
- Give each page one primary job. Separate tutorials, task guides, concepts,
  and exact reference material. Link to deeper detail instead of repeating it.
- Use descriptive headings, unique `title` and `description` frontmatter, and
  the appropriate `meta.json` entry. End task pages with a useful next step.

## Use a consistent page shape

| Page type | Required content |
| --- | --- |
| Overview or concept | Reader outcome, definition, where it fits, concrete example, boundaries, related task or reference. |
| Quickstart or tutorial | Working result, prerequisites and versions, starting directory and files, ordered steps, commands, expected output, verification, common failures, cleanup and next step. |
| Integration guide | Existing stack, supported versions and status, ownership boundaries, setup, smallest working request through the relevant layers, errors and security, verification, deployment implications. |
| Reference | Surface and version, exact syntax or API, parameters, types, defaults, required fields, outputs, errors, compatibility, verified examples. |
| Example application | Capability and status, public source, architecture, dependencies, setup and run commands, expected behavior, verification, limitations, cleanup, optional real screenshot. |

Adapt the shape to the task without adding empty boilerplate. Keep advanced
details in linked reference pages or accessible expandable sections.

## Verify every product claim

- Inspect the owning implementation and its meaningful tests before changing a
  command, API, language example, integration, or support statement. Never
  invent flags, commands, package names, framework support, or language syntax.
- Attach **Available**, **Alpha**, **Experimental**, or **Coming soon** to the
  precise capability and target. Keep
  `content/docs/introduction/support-matrix.mdx` consistent with the change.
- Separate local source builds, generated or simulated tests, installed public
  packages, mounted applications, and qualified deployment targets. Passing one
  does not qualify the others. Record exact versions and unverified boundaries.
- Keep Axiom Studio, Acode, and Axiom Marketplace labeled **Coming soon** until
  implementation and distribution evidence supports a status change. Do not
  give runnable installation instructions for a coming-soon product.
- Describe public capabilities rather than internal milestones, workstreams,
  completion ledgers, or cohort names. Keep qualification facts in readable
  support notes and reference tables.
- Do not publish internal implementation details, private architecture, internal
  source symbols, private test harnesses, or development history. Explain the behavior, public
  API, supported versions, and limits a developer needs to use the product.
  Keep implementation evidence in internal records.
- Maintainers with internal access must update the matching capability evidence
  when a claim changes. Without evidence or access, keep the claim bounded and
  report the unresolved verification. Never fabricate evidence or review dates.

## Make examples reproducible

- Declare the code-block language. Distinguish a runnable file, a partial
  snippet, conceptual pseudocode, and expected output. Identify the file being
  created or edited and the working directory for commands.
- Use filename captions and accurate syntax highlighting. Highlight only the
  lines a step changes or requires the reader to notice. Keep commands and
  expected output in separate blocks, retain copy controls, and use inline code
  for identifiers. Do not publish screenshots of code or oversized snippets
  when a shorter excerpt and a link to the complete file explain the task.
- Run the documented steps in a clean temporary workspace with the stated
  prerequisites. Exercise the claimed behavior, record the result internally,
  and document intentional failure cases and external dependencies.
- Use public, versioned dependencies and synthetic data. Examples for public
  readers must not require a private monorepo layout. Identify source-only
  developer paths explicitly instead of presenting them as installed workflows.
- Keep credentials, production DSNs, personal paths, signing material, private
  contracts, and real user data out of source, screenshots, and recorded output.
  Explain required environment variables with development placeholders.
- Explain destructive database steps and provide disposable local targets and
  cleanup instructions. Include applicable transaction, migration, permission,
  and recovery boundaries where the procedure depends on them.

## Add visuals that teach

- Use tables for comparisons, options, defaults, and support boundaries. Keep
  cells short and move long procedures into prose. Every table needs headers.
- Use diagrams for architecture, request/data flow, artifact lifecycles,
  ownership, and decisions. Keep labels consistent with the surrounding text
  and provide a text explanation of the same information.
- Use captioned `mermaid` fences for flowcharts, sequence diagrams, or ER
  diagrams. The registered build-time renderer creates local SVG images. Keep
  diagrams within its safe subset: no external resources, HTML, click handlers,
  directives, or custom styles. Verify MDX, static export, content security
  policy, and text-reader output. Reuse `ConceptDiagram` for its supported
  concepts. A highlighted code fence alone is not a rendered diagram.
- Keep diagrams readable in the existing dark theme and at narrow widths.
  Preserve keyboard access, accessible labels, reduced-motion behavior, and
  text alternatives. Do not communicate status through color alone.
- Capture screenshots from an application you actually ran. Store selected
  images under `public/images/docs/` with descriptive names, alt text, and a
  caption explaining what the image proves. Record the source revision,
  dependency versions, startup command, route, viewport, seed data, and capture
  steps internally. Use no generated mockups as evidence of working features.
- Keep code, commands, and important results selectable as text. A screenshot
  supplements instructions and verification; it cannot replace them.

## Validate and report

1. Run focused content validation with `pnpm lint:content` from this directory.
2. Before completing a public content or tooling change, run `pnpm check`.
   Follow `README.md` for locked dependency installation. When the change affects
   the static publishing path, also run `pnpm static:check`.
3. Run the exact changed example commands and relevant implementation checks.
   A documentation build alone does not verify application behavior.
4. Inspect changed public pages in the current dark theme at desktop and narrow
   widths. Check diagrams, tables, code overflow, links, keyboard navigation,
   search, heading anchors, and processed MDX/LLM text when affected.
5. Review the diff for lost information, unsupported claims, private data, em
   dashes in authored prose, and unnecessary changes to the visual design.
6. Report changed pages, preserved or corrected coverage, commands run, actual
   results, and unresolved limitations. Never describe an unrun check as passed.

These are contributor instructions, not a public product page. Keep private
plans and evidence in the internal docs repository. A request to prepare a plan
for review authorizes the guardrail and planning work; perform the broad public
rewrite after that plan has been reviewed. Publication follows the existing
release process and the user's authorization.
