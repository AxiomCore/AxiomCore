"use client";

import { useEffect, useId, useRef, useState } from "react";
import {
  Braces,
  FileCode2,
  Package,
  Layers,
  ScanSearch,
  ShieldCheck,
  Activity,
  GitCompareArrows,
  Pause,
  Play,
} from "lucide-react";

const diagrams = {
  contract: {
    title: "One contract. Connected tools.",
    caption: "Conceptual model · Choose a part to explore its boundary.",
    steps: [
      {
        name: "Source",
        icon: FileCode2,
        label: "Existing stack / Acore",
        title: "Start with the code you own.",
        text: "Extract a supported backend or author Acore declarations. Existing services and native frontend frameworks can keep their own implementation.",
        tags: ["Backend source", "Acore modules"],
        href: "/core-concepts/ghost-introspection",
        link: "Source boundaries",
      },
      {
        name: "Contract",
        icon: Package,
        label: "Versioned .axiom",
        title: "Make the interface explicit.",
        text: "The evaluated artifact carries typed models, operations, and declared policy. It describes a contract boundary; it does not contain the backend implementation.",
        tags: ["Models + operations", "Exact artifact bytes"],
        href: "/core-concepts/contracts-and-artifacts",
        link: "Contracts and artifacts",
      },
      {
        name: "Review",
        icon: ScanSearch,
        label: "Inspect / diff / test",
        title: "Understand what you are changing.",
        text: "Inspect declared relationships, compare contract versions, and run authored checks before updating consumers. Evidence stays tied to the artifact being reviewed.",
        tags: ["Semantic changes", "Authored checks"],
        href: "/inspector",
        link: "Explore Inspector",
      },
      {
        name: "Consume",
        icon: Layers,
        label: "Clients / Acore UI",
        title: "Connect the application at the boundary.",
        text: "Generate a supported client surface or consume contracts from Acore UI. Each runtime target has its own capabilities and lifecycle.",
        tags: ["Native clients", "Acore UI + .axiomapp"],
        href: "/introduction/support-matrix",
        link: "Target support",
      },
    ],
  },
  pipeline: {
    title: "From declaration to application.",
    caption: "Conceptual pipeline · Review gates still belong to your project.",
    steps: [
      {
        name: "Establish",
        icon: FileCode2,
        label: "Source boundary",
        title: "Choose what enters the contract.",
        text: "Author Acore, extract a supported backend, or bootstrap a domain model. Each source path has its own execution and availability boundary.",
        tags: ["Authored", "Extracted / bootstrapped"],
        href: "#1-establish-the-source-boundary",
        link: "Source boundary",
      },
      {
        name: "Evaluate",
        icon: Braces,
        label: "Parse / validate",
        title: "Resolve declarations into typed data.",
        text: "The parser, evaluator, standard library, and validators produce a typed in-memory representation. Run domain, security, and authored checks as appropriate.",
        tags: ["Composition", "Validation"],
        href: "#2-evaluate-and-validate-acore",
        link: "Evaluation and checks",
      },
      {
        name: "Package",
        icon: Package,
        label: "Build / review",
        title: "Review the exact artifact.",
        text: "Build a local .axiom artifact and lock information. Compare semantic changes and verify provenance before updating consumers. Local builds can be unsigned.",
        tags: ["Artifact + lock", "Diff + provenance"],
        href: "#3-build-the-contract-artifact",
        link: "Build and review",
      },
      {
        name: "Consume",
        icon: Layers,
        label: "Pull / execute",
        title: "Run through the selected target.",
        text: "Pull a supported client surface and load the contract through its runtime. Native and browser targets share the contract model, with different host capabilities.",
        tags: ["Generated client", "Target runtime"],
        href: "#5-pull-a-client-surface",
        link: "Pull and execute",
      },
    ],
  },
  inspector: {
    title: "Evidence becomes an explorable model.",
    caption:
      "Conceptual model · This illustration is not a live runtime trace.",
    steps: [
      {
        name: "Inputs",
        icon: FileCode2,
        label: "Source / artifacts",
        title: "Begin with available evidence.",
        text: "Acore source, verified contracts, locks, packages, authority, and captured runtime events contribute different evidence. Missing evidence remains explicit.",
        tags: ["Source spans", "Artifact identities"],
        href: "/inspector/evidence-and-relationships",
        link: "Evidence sources",
      },
      {
        name: "Facts",
        icon: Braces,
        label: "Semantic identities",
        title: "Give each fact a stable identity.",
        text: "Pages, operations, fields, permissions, and observed invocations become semantic facts with a truth layer, revision, and supporting references.",
        tags: ["Facts + revisions", "Evidence references"],
        href: "/inspector/evidence-and-relationships#fact",
        link: "Understand facts",
      },
      {
        name: "Connections",
        icon: ScanSearch,
        label: "Typed relationships",
        title: "Follow what connects the pieces.",
        text: "Typed edges describe dependencies, calls, authority, and bounded impact. Static reachability alone does not establish that a call actually happened.",
        tags: ["Dependencies", "Bounded impact"],
        href: "/inspector/trace-and-explain",
        link: "Explore relationships",
      },
      {
        name: "Views",
        icon: Layers,
        label: "CLI / agents / dashboard",
        title: "Query the same evidence engine.",
        text: "The CLI, machine formats, and local dashboard present the same semantic identities and graph revision. An AI explanation is not a new source of facts.",
        tags: ["Machine-readable", "Human-readable"],
        href: "/inspector/local-dashboard",
        link: "Open the local dashboard",
      },
    ],
  },
  evidence: {
    title: "Four layers. Four different questions.",
    caption: "Evidence layers · These are distinct views, not execution steps.",
    steps: [
      {
        name: "Declared",
        icon: FileCode2,
        label: "What was authored?",
        title: "A request is a declaration.",
        text: "An extension asks to write cart.total_label. That declaration describes requested authority; it does not grant access.",
        tags: ["Source", "Requested permission"],
        href: "/inspector/security-and-authority",
        link: "Read authority boundaries",
      },
      {
        name: "Resolved",
        icon: ShieldCheck,
        label: "What is effective?",
        title: "Resolve the request against policy.",
        text: "The application grant and target policy determine effective authority. A resolved permission says what is allowed, not what was used.",
        tags: ["Application grant", "Target policy"],
        href: "/inspector/security-and-authority",
        link: "Review effective authority",
      },
      {
        name: "Observed",
        icon: Activity,
        label: "What actually happened?",
        title: "An execution needs runtime evidence.",
        text: "A captured invocation proposed a patch and the host accepted it. This supports a claim about that execution, not every possible execution.",
        tags: ["Captured session", "Runtime event"],
        href: "/inspector/runtime-and-audits",
        link: "Runtime evidence",
      },
      {
        name: "Changed",
        icon: GitCompareArrows,
        label: "What is different?",
        title: "Compare compatible snapshots.",
        text: "A new permission or removed operation is a semantic change. Impact follows bounded, evidenced relationships rather than guessed implementation behavior.",
        tags: ["Before / after", "Bounded impact"],
        href: "/inspector/changes-impact-and-ci",
        link: "Change and impact",
      },
    ],
  },
};

export function ConceptDiagram({ kind }: { kind: keyof typeof diagrams }) {
  const diagram = diagrams[kind];
  const [active, setActive] = useState(0);
  const [playing, setPlaying] = useState(true);
  const [reducedMotion, setReducedMotion] = useState(true);
  const [visible, setVisible] = useState(false);
  const ref = useRef<HTMLElement>(null);
  const id = useId();
  const step = diagram.steps[active];

  useEffect(() => {
    const preference = window.matchMedia("(prefers-reduced-motion: reduce)");
    const update = () => setReducedMotion(preference.matches);
    update();
    preference.addEventListener("change", update);
    const observer = new IntersectionObserver(
      ([entry]) => setVisible(entry.isIntersecting),
      { threshold: 0.3 },
    );
    if (ref.current) observer.observe(ref.current);
    return () => {
      preference.removeEventListener("change", update);
      observer.disconnect();
    };
  }, []);

  const running = playing && visible && !reducedMotion;
  useEffect(() => {
    if (!running) return;
    const timer = window.setInterval(() => {
      if (!document.hidden)
        setActive((current) => (current + 1) % diagram.steps.length);
    }, 6500);
    return () => window.clearInterval(timer);
  }, [running, diagram.steps.length]);

  return (
    <figure
      ref={ref}
      className="concept-diagram not-prose"
      aria-labelledby={`${id}-title`}
      data-running={running}
    >
      <div className="concept-heading">
        <span id={`${id}-title`} className="docs-label">
          {diagram.title}
        </span>
        {!reducedMotion && (
          <button
            type="button"
            className="concept-motion"
            onClick={() => setPlaying(!playing)}
            aria-label={playing ? "Pause illustration" : "Play illustration"}
          >
            {playing ? <Pause size={12} /> : <Play size={12} />}
            {playing ? "Pause" : "Play"}
          </button>
        )}
      </div>
      <div
        className="concept-nodes"
        aria-label="Explore this concept"
        onFocus={() => setPlaying(false)}
      >
        {diagram.steps.map((item, index) => {
          const Icon = item.icon;
          return (
            <button
              type="button"
              key={item.name}
              className="concept-node"
              aria-pressed={active === index}
              aria-controls={`${id}-detail`}
              onClick={() => {
                setActive(index);
                setPlaying(false);
              }}
            >
              <span className="concept-number">0{index + 1}</span>
              <Icon size={24} strokeWidth={1.25} aria-hidden="true" />
              <span className="concept-name">{item.name}</span>
              <span className="concept-node-label">{item.label}</span>
              <span className="concept-signal" aria-hidden="true" />
            </button>
          );
        })}
      </div>
      <div
        id={`${id}-detail`}
        className="concept-detail"
        aria-live={playing ? "off" : "polite"}
      >
        <div className="concept-detail-copy">
          <p className="concept-detail-title">{step.title}</p>
          <p>{step.text}</p>
          <a href={step.href} className="concept-link">
            {step.link} <span aria-hidden="true">↗</span>
          </a>
        </div>
        <div className="concept-tags" aria-label="Key concepts">
          {step.tags.map((tag) => (
            <span key={tag}>{tag}</span>
          ))}
        </div>
      </div>
      <figcaption>{diagram.caption}</figcaption>
    </figure>
  );
}
