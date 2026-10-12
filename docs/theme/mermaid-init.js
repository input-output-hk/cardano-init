// Render ```mermaid code blocks without the mdbook-mermaid preprocessor:
// load Mermaid from a CDN and swap each block for its rendered diagram.
(async () => {
  const blocks = document.querySelectorAll("code.language-mermaid");
  if (blocks.length === 0) return;

  const { default: mermaid } = await import(
    "https://cdn.jsdelivr.net/npm/mermaid@11.4.1/dist/mermaid.esm.min.mjs"
  );
  const dark = ["coal", "navy", "ayu"].some((t) =>
    document.documentElement.classList.contains(t)
  );
  mermaid.initialize({ startOnLoad: false, theme: dark ? "dark" : "default" });

  blocks.forEach((code) => {
    const div = document.createElement("div");
    div.className = "mermaid";
    div.textContent = code.textContent;
    code.parentElement.replaceWith(div);
  });
  await mermaid.run({ querySelector: ".mermaid" });
})();
