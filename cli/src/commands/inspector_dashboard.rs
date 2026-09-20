pub(super) const HTML: &str = r##"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width,initial-scale=1">
  <meta name="color-scheme" content="dark">
  <title>Axiom Inspector</title>
  <link rel="stylesheet" href="/assets/inspector.css">
</head>
<body>
  <div id="loading" class="loading"><span></span>Resolving application evidence…</div>
  <header class="topbar">
    <a class="brand" href="#overview" aria-label="Axiom Inspector overview"><i>A</i><span>Axiom <b>Inspector</b></span></a>
    <label class="search"><span>⌕</span><input id="search" type="search" placeholder="Search facts, source, capabilities…" aria-label="Search application evidence"><kbd>/</kbd></label>
    <div class="selectors">
      <label>Target <select id="target"><option value="">All targets</option></select></label>
      <label>Truth <select id="layer"><option value="">All layers</option><option>declared</option><option>resolved</option><option>observed</option></select></label>
    </div>
  </header>
  <div class="shell">
    <nav id="nav" aria-label="Inspector views">
      <a href="#overview" data-view="overview">Overview</a>
      <a href="#graph" data-view="graph">Graph</a>
      <span>Explore</span>
      <a href="#source" data-view="source">Source</a>
      <a href="#ui" data-view="ui">UI & state</a>
      <a href="#backend" data-view="backend">Backend</a>
      <a href="#extensions" data-view="extensions">Extensions</a>
      <a href="#dependencies" data-view="dependencies">Dependencies</a>
      <span>Assurance</span>
      <a href="#security" data-view="security">Security</a>
      <a href="#runtime" data-view="runtime">Runtime</a>
      <a href="#live" data-view="live">Live</a>
      <a href="#changes" data-view="changes">Changes</a>
      <a href="#ask" data-view="ask">Ask Axiom</a>
      <footer><b id="connection">● Connected</b><small id="revision"></small></footer>
    </nav>
    <main id="main" tabindex="-1"></main>
    <aside id="drawer" aria-label="Evidence details" aria-hidden="true"></aside>
  </div>
  <div id="toast" role="status" aria-live="polite"></div>
  <script src="/assets/inspector.js" defer></script>
</body>
</html>"##;

pub(super) const CSS: &str = include_str!("inspector_dashboard.css");
pub(super) const JS: &str = include_str!("inspector_dashboard.js");

#[cfg(test)]
mod tests {
    use super::JS;

    #[test]
    fn dashboard_javascript_parses_when_node_is_available() {
        let Ok(version) = std::process::Command::new("node").arg("--version").output() else {
            return;
        };
        if !version.status.success() {
            return;
        }
        let path = std::env::temp_dir().join(format!(
            "axiom-inspector-dashboard-{}-{}.js",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, JS).unwrap();
        let output = std::process::Command::new("node")
            .arg("--check")
            .arg(&path)
            .output()
            .unwrap();
        std::fs::remove_file(path).unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
