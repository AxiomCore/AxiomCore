/** Turn authored Mermaid fences into server-rendered, accessible illustrations. */
export default function remarkMermaid() {
  return (tree) => {
    function visit(node) {
      if (!node.children) return;
      node.children = node.children.map((child) => {
        if (child.type === 'code' && child.lang === 'mermaid') {
          const caption = child.meta?.match(/title="([^"]+)"/)?.[1];
          if (!caption) throw new Error('Mermaid fences need a descriptive title="..." caption.');
          return {
            type: 'mdxJsxFlowElement',
            name: 'Mermaid',
            attributes: [
              { type: 'mdxJsxAttribute', name: 'source', value: child.value },
              { type: 'mdxJsxAttribute', name: 'caption', value: caption },
            ],
            children: [],
          };
        }
        visit(child);
        return child;
      });
    }
    visit(tree);
  };
}
