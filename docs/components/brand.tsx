/** Same mark, proportions, and wordmark as the landing page and Playground. */
export function Brand() {
  return (
    <span className="axiom-logo">
      <svg
        aria-hidden="true"
        focusable="false"
        viewBox="0 0 32 32"
        className="axiom-logo-mark"
      >
        <rect className="logo-frame" height="31" width="31" x=".5" y=".5" />
        <path className="logo-glyph" d="M7 24 16 7l9 17M11.5 18.5h9" />
        <rect className="logo-core" height="5" width="5" x="23" y="4" />
      </svg>
      <span className="axiom-logo-word">
        <b>Axiom</b>
        <span>Core</span>
      </span>
    </span>
  );
}
