import defaultMdxComponents from "fumadocs-ui/mdx";
import type { MDXComponents } from "mdx/types";
import { ConceptDiagram } from "@/components/docs/concept-diagram";
import {
  CapabilityTable,
  StatusBadge,
  StatusNote,
} from "@/components/docs/availability";

export function getMDXComponents(components?: MDXComponents): MDXComponents {
  return {
    ...defaultMdxComponents,
    ConceptDiagram,
    CapabilityTable,
    StatusBadge,
    StatusNote,
    ...components,
  };
}
