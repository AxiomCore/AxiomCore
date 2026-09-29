import { cpSync, mkdirSync } from "node:fs";
import { resolve, join } from "node:path";

// Commit the snapshot: docs releases are built from this repository alone.
// Run explicitly when the approved frontend design system changes, never at build time.
const docsRoot = resolve(import.meta.dirname, "..");
const source = resolve(
  process.argv[2] ?? join(docsRoot, "../../axiom-frontend/lib/src"),
);
const destination = join(docsRoot, "lib/brand");
mkdirSync(join(destination, "styles"), { recursive: true });
mkdirSync(join(destination, "assets"), { recursive: true });
for (const file of ["theme.css", "fonts.css", "analytics.css"]) {
  cpSync(join(source, "styles", file), join(destination, "styles", file));
}
cpSync(join(source, "assets/fonts"), join(destination, "assets/fonts"), {
  recursive: true,
});
console.log("Synced the approved AxiomCore theme and local fonts.");

mkdirSync(join(destination, "analytics"), { recursive: true });
cpSync(join(source, "analytics/client.ts"), join(destination, "analytics/client.ts"));
