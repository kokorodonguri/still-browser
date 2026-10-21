// Thin capsule scrollbars for web pages. A cascade layer keeps any site's own
// scrollbar styling in charge; forced-colors mode keeps the system scrollbars.
(() => {
  try {
    const sheet = new CSSStyleSheet();
    sheet.replaceSync(`@layer still-scrollbars { @media not (forced-colors: active) {
      ::-webkit-scrollbar { width: 10px; height: 10px; background: transparent; }
      ::-webkit-scrollbar-track, ::-webkit-scrollbar-corner { background: transparent; }
      ::-webkit-scrollbar-button { display: none; width: 0; height: 0; }
      ::-webkit-scrollbar-thumb { background: rgb(142 142 147 / 42%); background-clip: padding-box; border: 3px solid transparent; border-radius: 10px; min-height: 28px; }
      ::-webkit-scrollbar-thumb:hover { background-color: rgb(142 142 147 / 65%); border-width: 2px; }
      ::-webkit-scrollbar-thumb:active { background-color: rgb(142 142 147 / 80%); border-width: 2px; }
    } }`);
    document.adoptedStyleSheets = [...document.adoptedStyleSheets, sheet];
  } catch (_) {}
})();
