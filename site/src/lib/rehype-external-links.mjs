// Links that leave the site open in a new tab: other domains, and addresses
// on the reader's own network such as http://openstreamer.local or 10.0.0.1

function isExternal(href) {
  if (!/^https?:\/\//i.test(href)) return false;
  try {
    return Boolean(new URL(href).hostname);
  } catch {
    return false;
  }
}

export default function rehypeExternalLinks() {
  const walk = (node) => {
    if (
      node.type === "element" &&
      node.tagName === "a" &&
      typeof node.properties?.href === "string" &&
      isExternal(node.properties.href)
    ) {
      node.properties.target = "_blank";
      node.properties.rel = ["noopener", "noreferrer"];
    }
    node.children?.forEach(walk);
  };
  return (tree) => walk(tree);
}
