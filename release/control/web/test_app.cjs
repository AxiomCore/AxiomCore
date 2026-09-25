const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

function classes(...names) {
  const values = new Set(names);
  return {
    toggle(name, force) {
      if (force === undefined) force = !values.has(name);
      if (force) values.add(name);
      else values.delete(name);
      return values.has(name);
    },
    add(name) { values.add(name); },
    remove(name) { values.delete(name); },
    contains(name) { return values.has(name); },
  };
}

function dashboard() {
  const elements = new Map();
  const element = (selector) => {
    if (!elements.has(selector)) elements.set(selector, {
      classList: classes(), textContent: "", innerHTML: "", scrollTop: 0,
      clientHeight: 100, scrollHeight: 100, placeholder: "",
    });
    return elements.get(selector);
  };
  const context = vm.createContext({
    document: {querySelector: (selector) => selector === 'meta[name="release-token"]' ? {content: "test"} : element(selector)},
    refreshCount: 0,
  });
  const source = fs.readFileSync(path.join(__dirname, "app.js"), "utf8");
  vm.runInContext(source.slice(0, source.indexOf('$("#refresh").addEventListener')), context);
  vm.runInContext("loadState = () => { refreshCount++; };", context);
  return {context, element};
}

test("completed follow-up run refreshes state only on the status transition", () => {
  const {context} = dashboard();
  const job = {trainId: "2026.09.25.4", status: "complete", startedAt: new Date().toISOString(),
    order: [], completed: [], logTail: "", error: null};
  context.job = job;
  vm.runInContext('snapshot = {trainId: "2026.09.25.5"}; renderJob(job);', context);
  assert.equal(context.refreshCount, 0);
  vm.runInContext('currentJob = {...job, status: "running"}; renderJob(job);', context);
  assert.equal(context.refreshCount, 1);
  vm.runInContext("renderJob(job);", context);
  assert.equal(context.refreshCount, 1);
});

test("Inspect stays expanded after source state rerenders", () => {
  const {context, element} = dashboard();
  const details = {classList: classes("hidden")};
  const inspect = {addEventListener: (_, handler) => { inspect.click = handler; },
    setAttribute: (name, value) => { inspect[name] = value; }};
  const card = {dataset: {repo: "AxiomCore"},
    querySelector: (selector) => ({".repo-details": details, ".inspect-button": inspect})[selector] || null,
    querySelectorAll: () => []};
  const list = element("#repo-list");
  list.querySelector = () => null;
  list.querySelectorAll = (selector) => selector === ".repo" ? [card] : [];
  vm.runInContext('snapshot = {repositories: [{name: "AxiomCore", files: [{path: "file.txt", status: " M"}], ahead: 0}]}; renderRepos();', context);
  inspect.click();
  assert.equal(inspect["aria-expanded"], "true");
  vm.runInContext("renderRepos();", context);
  assert.match(list.innerHTML, /class="repo-details "/);
  assert.match(list.innerHTML, /aria-expanded="true"/);
  inspect.click();
  vm.runInContext("renderRepos();", context);
  assert.match(list.innerHTML, /class="repo-details hidden"/);
  assert.match(list.innerHTML, /aria-expanded="false"/);
});
