import defaultMdxComponents from 'fumadocs-ui/mdx';
import type { MDXComponents } from 'mdx/types';
import {
  CapabilityTable,
  StatusBadge,
  StatusNote,
} from '@/components/docs/availability';

export function getMDXComponents(components?: MDXComponents): MDXComponents {
  return {
    ...defaultMdxComponents,
    CapabilityTable,
    StatusBadge,
    StatusNote,
    ...components,
  };
}
