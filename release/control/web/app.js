"use strict";

const token = document.querySelector('meta[name="release-token"]').content;
const $ = (selector) => document.querySelector(selector);
const escapeHtml = (value) => String(value ?? "").replace(/[&<>"']/g, (char) => ({"&":"&amp;","<":"&lt;",">":"&gt;","\"":"&quot;","'":"&#39;"})[char]);
let snapshot = null;
let draft = {};
let preview = null;
let currentJob = null;
let polling = null;

async function api(path, value) {
  const options = {headers: {"X-Axiom-Release-Token": token}, cache: "no-store"};
  if (value !== undefined) {
    options.method = "POST";
    options.headers["Content-Type"] = "application/json";
    options.body = JSON.stringify(value);
  }
  const response = await fetch(path, options);
  let body;
  try { body = await response.json(); } catch { throw new Error(`Unexpected server response (${response.status})`); }
  if (!response.ok) throw new Error(body.error || `Request failed (${response.status})`);
  return body;
}

function notice(message, kind = "warning") {
  const box = $("#notice");
  box.textContent = message;
  box.className = `notice ${kind}`;
  box.scrollIntoView({behavior: "smooth", block: "nearest"});
}

function clearNotice() { $("#notice").className = "notice hidden"; }
function invalidatePreview() { preview = null; $("#preview-empty").classList.remove("hidden"); $("#preview-content").classList.add("hidden"); }

function componentDraft(component) {
  if (!draft[component.id]) draft[component.id] = {
    selected: false, type: "feature", summary: "", migration: "",
    version: component.candidateVersion || component.sourceVersion || ""
  };
  return draft[component.id];
}

function generatedChange(component, form) {
  const name = component.id.replaceAll("-", " ");
  const version = component.versioned && form.version.trim() ? ` in ${form.version.trim()}` : "";
  const action = {
    feature: `Add ${name} improvements${version}.`,
    fix: `Fix issues in ${name}${version}.`,
    security: `Strengthen ${name} security${version}.`,
    breaking: `Introduce breaking changes to ${name}${version}.`,
    internal: `Improve ${name} internals${version}.`
  }[form.type] || `Update ${name}${version}.`;
  return action;
}

function selectedComponents() {
  return snapshot.components.filter((component) => componentDraft(component).selected);
}

function generatedSummary(components) {
  const introduction = components.length === 1 ? "Release update: " : `Release ${components.length} Axiom components: `;
  let result = introduction;
  for (let index = 0; index < components.length; index++) {
    const component = components[index];
    const note = componentDraft(component).summary.trim() || generatedChange(component, componentDraft(component));
    const addition = `${index ? "; " : ""}${note}`;
    const remaining = components.length - index - 1;
    const suffix = remaining ? `; and ${remaining} more component${remaining === 1 ? "" : "s"}.` : "";
    if (result.length + addition.length + suffix.length > 300) {
      if (index === 0) {
        const room = 300 - result.length - suffix.length - 1;
        result += `${note.slice(0, room).trimEnd()}…${suffix}`;
      } else {
        const count = components.length - index;
        result += `; and ${count} more component${count === 1 ? "" : "s"}.`;
      }
      break;
    }
    result += addition;
  }
  return result.slice(0, 300);
}

function syncSelectionControls() {
  const available = snapshot.components.filter((component) => component.localBuild);
  const selected = available.filter((component) => componentDraft(component).selected).length;
  const selectAll = $("#select-all-components");
  selectAll.disabled = available.length === 0;
  selectAll.checked = available.length > 0 && selected === available.length;
  selectAll.indeterminate = selected > 0 && selected < available.length;
  $("#generate-all-changes").disabled = selected === 0;
  $("#selection-count").textContent = `${selected} selected`;
}

function renderComponents() {
  const list = $("#component-list");
  list.innerHTML = snapshot.components.map((component) => {
    const form = componentDraft(component);
    const checked = form.selected ? "checked" : "";
    const disabled = component.localBuild ? "" : "disabled";
    const label = component.localBuild ? ({published:"Published",staged:"Staged",stale:"Stale candidate",incomplete:"Interrupted"}[component.state] || component.builder) : "Builder gap";
    return `<article class="component ${form.selected ? "selected" : ""}" data-id="${escapeHtml(component.id)}">
      <div class="component-head"><input class="component-check" type="checkbox" aria-label="Release ${escapeHtml(component.id)}" ${checked} ${disabled}>
      <div class="component-title"><div class="component-name">${escapeHtml(component.id)}</div><div class="component-meta">${escapeHtml(component.destination)} · Source ${escapeHtml(component.sourceVersion || "digest")} · Last released ${escapeHtml(component.lastReleased || "unknown")}${component.dependsOn.length ? ` · Needs ${escapeHtml(component.dependsOn.join(", "))}` : ""}</div></div>
      <span class="component-state ${component.localBuild ? component.state === "stale" ? "stale" : "" : "unavailable"}" title="${escapeHtml(component.candidateError || "")}">${label}</span></div>
      <div class="component-fields"><div class="field-grid"><div><label>Change type</label><select class="select-input change-type" aria-label="Change type for ${escapeHtml(component.id)}">${["feature","fix","security","breaking","internal"].map((type) => `<option value="${type}" ${form.type === type ? "selected" : ""}>${type[0].toUpperCase() + type.slice(1)}</option>`).join("")}</select></div>
      ${component.versioned ? `<div><label>Candidate version</label><input class="text-input candidate-version" aria-label="Candidate version for ${escapeHtml(component.id)}" value="${escapeHtml(form.version)}" placeholder="X.Y.Z" autocomplete="off"></div>` : `<div><label>Release identity</label><div class="component-meta">Immutable digest / deployment</div></div>`}
      <div class="note-wrap"><label>What changed in ${escapeHtml(component.id)}?</label><div class="note-row"><input class="text-input change-summary" aria-label="Change summary for ${escapeHtml(component.id)}" maxlength="500" value="${escapeHtml(form.summary)}" placeholder="A precise, user-facing change" autocomplete="off"><button class="mini-button template-button" type="button" title="Replace this component's change note with a template">Generate change</button></div></div>
      <div class="note-wrap migration-wrap ${form.type === "breaking" ? "" : "hidden"}"><label>Migration guidance</label><input class="text-input migration" aria-label="Migration guidance for ${escapeHtml(component.id)}" value="${escapeHtml(form.migration)}" placeholder="What must users change?" autocomplete="off"></div>
      </div></div></article>`;
  }).join("");
  list.querySelectorAll(".component").forEach((card) => {
    const id = card.dataset.id;
    const form = draft[id];
    card.querySelector(".component-check").addEventListener("change", (event) => {
      const enabled = event.target.checked;
      if (id.startsWith("ui-host-")) {
        for (const host of ["ui-host-web", "ui-host-android", "ui-host-ios"]) {
          const hostComponent = snapshot.components.find((item) => item.id === host);
          if (hostComponent?.localBuild) componentDraft(hostComponent).selected = enabled;
        }
      } else form.selected = enabled;
      invalidatePreview(); renderComponents();
    });
    card.querySelector(".change-type").addEventListener("change", (event) => { form.type = event.target.value; card.querySelector(".migration-wrap").classList.toggle("hidden", form.type !== "breaking"); invalidatePreview(); });
    card.querySelector(".change-summary").addEventListener("input", (event) => { form.summary = event.target.value; invalidatePreview(); });
    card.querySelector(".migration").addEventListener("input", (event) => { form.migration = event.target.value; invalidatePreview(); });
    const version = card.querySelector(".candidate-version");
    if (version) version.addEventListener("input", (event) => { form.version = event.target.value; invalidatePreview(); });
    card.querySelector(".template-button").addEventListener("click", () => {
      form.summary = generatedChange(snapshot.components.find((item) => item.id === id), form);
      card.querySelector(".change-summary").value = form.summary;
      invalidatePreview();
    });
  });
  syncSelectionControls();
}

function untrackedDirectory(file) {
  return file.status === "??" && file.path.endsWith("/");
}

function generatedCommitMessage(repository, paths) {
  const scope = repository.toLowerCase().replace(/[^a-z0-9-]+/g, "-").slice(0, 40);
  const change = paths.length === 1 ? `update ${paths[0]}` : `update ${paths.length} selected files`;
  return `chore(${scope}): ${change}`.slice(0, 200);
}

function renderRepos() {
  const list = $("#repo-list");
  const dirty = snapshot.repositories.filter((repo) => repo.files.length || repo.error);
  const ahead = snapshot.repositories.filter((repo) => !repo.files.length && !repo.error && repo.ahead);
  const pushSummary = ahead.length ? `<div class="push-summary"><strong>Committed source awaiting push</strong><p>These repositories have no reviewable uncommitted files. A confirmed dashboard release pushes the required commits before building; this panel does not push on its own.</p><div>${ahead.map((repo) => `<span>${escapeHtml(repo.name)} · ${repo.ahead} ahead</span>`).join("")}</div></div>` : "";
  if (!dirty.length) { list.innerHTML = `<div class="empty-run">No uncommitted release-source changes to review.</div>${pushSummary}`; return; }
  list.innerHTML = dirty.map((repo) => {
    const selectable = repo.files.filter((file) => !untrackedDirectory(file));
    return `<article class="repo" data-repo="${escapeHtml(repo.name)}"><div class="repo-top"><div><div class="repo-name">${escapeHtml(repo.name)}</div><div class="repo-info">${repo.files.length} changed path(s) · ${repo.ahead || 0} commit(s) ahead${repo.error ? ` · ${escapeHtml(repo.error)}` : ""}</div></div><button class="subtle-button inspect-button" type="button">Inspect</button></div><div class="repo-details hidden">${selectable.length ? `<label class="repo-select-all"><input class="repo-select-all-check" type="checkbox" aria-label="Select all reviewable files in ${escapeHtml(repo.name)}"><span>Select all files</span><small class="selected-file-count">0 / ${selectable.length} selected</small></label>` : ""}<div class="files">${repo.files.map((file) => `<label class="file-row ${untrackedDirectory(file) ? "file-unavailable" : ""}"><input class="file-check" type="checkbox" value="${escapeHtml(file.path)}" ${untrackedDirectory(file) ? 'disabled title="Untracked directory; review in its owning repository"' : ""}><button type="button" class="diff-button" data-path="${escapeHtml(file.path)}">${escapeHtml(file.status)} ${escapeHtml(file.path)}</button>${untrackedDirectory(file) ? '<small>Directory — review separately</small>' : ""}</label>`).join("") || '<div class="repo-info">No uncommitted files. Push happens in the confirmed release run.</div>'}</div><pre class="diff hidden"></pre>${selectable.length ? `<div class="repo-actions"><div class="repo-message-row"><input class="text-input commit-message" aria-label="Commit message for ${escapeHtml(repo.name)}" placeholder="Commit message for selected files" maxlength="200"><button class="mini-button generate-commit-message" type="button">Generate message</button></div><button class="secondary-button commit-button" type="button">Commit selected</button></div>` : ""}</div></article>`;
  }).join("") + pushSummary;
  list.querySelectorAll(".repo").forEach((card) => {
    const repo = card.dataset.repo;
    const details = card.querySelector(".repo-details");
    card.querySelector(".inspect-button").addEventListener("click", () => details.classList.toggle("hidden"));
    const fileChecks = [...card.querySelectorAll(".file-check:not(:disabled)")];
    const selectAll = card.querySelector(".repo-select-all-check");
    function syncFileSelection() {
      if (!selectAll) return;
      const selected = fileChecks.filter((input) => input.checked).length;
      selectAll.checked = selected === fileChecks.length;
      selectAll.indeterminate = selected > 0 && selected < fileChecks.length;
      card.querySelector(".selected-file-count").textContent = `${selected} / ${fileChecks.length} selected`;
    }
    if (selectAll) selectAll.addEventListener("change", () => {
      fileChecks.forEach((input) => { input.checked = selectAll.checked; });
      syncFileSelection();
    });
    fileChecks.forEach((input) => input.addEventListener("change", syncFileSelection));
    card.querySelectorAll(".diff-button").forEach((button) => button.addEventListener("click", async () => {
      try {
        const item = await api(`/api/diff?repo=${encodeURIComponent(repo)}&path=${encodeURIComponent(button.dataset.path)}`);
        const target = card.querySelector(".diff"); target.textContent = item.diff || "No textual diff available."; target.classList.remove("hidden");
      } catch (error) { notice(error.message, "error"); }
    }));
    const generate = card.querySelector(".generate-commit-message");
    if (generate) generate.addEventListener("click", () => {
      const paths = fileChecks.filter((input) => input.checked).map((input) => input.value);
      if (!paths.length) { notice(`Select files in ${repo} before generating a commit message.`); return; }
      card.querySelector(".commit-message").value = generatedCommitMessage(repo, paths);
    });
    const commit = card.querySelector(".commit-button");
    if (commit) commit.addEventListener("click", async () => {
      const paths = [...card.querySelectorAll(".file-check:checked")].map((input) => input.value);
      const message = card.querySelector(".commit-message").value;
      if (!paths.length || !message.trim()) { notice("Select files and enter a commit message first.", "error"); return; }
      if (!confirm(`Commit only ${paths.length} selected file(s) in ${repo}?`)) return;
      try { commit.disabled = true; await api("/api/commit", {repository: repo, paths, message}); notice(`Committed selected changes in ${repo}. Review the release plan again.`, "success"); invalidatePreview(); await loadState(); }
      catch (error) { notice(error.message, "error"); }
      finally { commit.disabled = false; }
    });
  });
}

function formValue() {
  return {summary: $("#release-summary").value.trim(), components: snapshot.components.filter((item) => draft[item.id]?.selected).map((item) => ({
    id: item.id, type: draft[item.id].type, summary: draft[item.id].summary.trim(),
    migration: draft[item.id].migration.trim(), ...(item.versioned ? {version: draft[item.id].version.trim()} : {})
  }))};
}

function renderPreview(value) {
  preview = value;
  $("#preview-empty").classList.add("hidden");
  const box = $("#preview-content");
  box.classList.remove("hidden");
  box.innerHTML = `<div class="preview-key"><span>New train</span><strong>${escapeHtml(value.trainId)}</strong></div>
    <div class="preview-key"><span>Selected</span><strong>${value.components.length} components</strong></div>
    <div class="preview-key"><span>Version mirrors</span><strong>${value.versionFiles.length}</strong></div>
    <div class="preview-key"><span>Changelog notes</span><strong>${value.notes.length}</strong></div>
    <div class="preview-key"><span>Storage</span><strong>${escapeHtml(value.storage)}</strong></div>
    <details class="source-details"><summary>Source branches to verify and push (${value.repositories.length})</summary><ul>${value.repositories.map((repo) => `<li><strong>${escapeHtml(repo.name)}</strong> · ${escapeHtml(repo.upstream || "no upstream")} · ${escapeHtml(repo.head.slice(0, 12))} · ${repo.ahead} ahead${repo.commitsToPush?.length ? `<ul>${repo.commitsToPush.map((item) => `<li>${escapeHtml(item)}</li>`).join("")}</ul>` : ""}</li>`).join("")}</ul></details>
    <ol class="sequence">${["Create train & back up intent", "Prepare versions and notes", "Commit managed files", "Push exact source commits", ...value.order.map((id) => `Build ${id}`), ...value.order.map((id) => `Publish & verify ${id}`)].map((item, index) => `<li><b>${index + 1}</b>${escapeHtml(item)}</li>`).join("")}</ol>
    ${value.blockers.length ? `<div class="blockers"><strong>Resolve before release</strong><ul>${value.blockers.map((item) => `<li>${escapeHtml(item)}</li>`).join("")}</ul></div>` : '<div class="ready-box">Ready to run. If any step fails, work stops and evidence remains intact for inspection or resume.</div>'}
    ${value.ready ? `<div class="confirm-box"><strong>Production confirmation</strong><p>This commits release metadata, pushes the required branches, and publishes to production. There is no automatic rollback after a partial publication. Type <span class="confirmation-code">PUBLISH ${escapeHtml(value.trainId)}</span>.</p><input id="start-confirm" class="text-input" autocomplete="off" aria-label="Production confirmation"><button id="start-button" class="primary-button" type="button">Run complete release</button></div>` : ""}`;
  if (value.ready) $("#start-button").addEventListener("click", startRelease);
  box.scrollIntoView({behavior: "smooth", block: "start"});
}

async function startRelease() {
  if (!preview) return;
  const button = $("#start-button"); button.disabled = true;
  try {
    const confirmation = $("#start-confirm").value;
    const result = await api("/api/start", {previewId: preview.previewId, confirmation});
    notice(`Started release ${result.trainId}. Keep the dashboard open to watch progress.`, "success");
    await loadJob(result.trainId);
  } catch (error) { notice(error.message, "error"); button.disabled = false; }
}

function elapsed(started) {
  const seconds = Math.max(0, Math.floor((Date.now() - Date.parse(started)) / 1000));
  const hours = Math.floor(seconds / 3600), minutes = Math.floor((seconds % 3600) / 60);
  return `${String(hours).padStart(2, "0")}:${String(minutes).padStart(2, "0")}:${String(seconds % 60).padStart(2, "0")}`;
}

function renderJob(job) {
  currentJob = job;
  $("#job-empty").classList.toggle("hidden", Boolean(job));
  $("#job-content").classList.toggle("hidden", !job);
  if (!job) return;
  const pill = $("#run-state"); pill.textContent = job.status;
  pill.className = `pill ${job.status === "complete" ? "" : job.status === "running" ? "warning" : "error"}`;
  $("#job-train").textContent = job.trainId;
  $("#job-elapsed").textContent = elapsed(job.startedAt);
  const names = ["cycle", "prepare", "commit", "push", ...job.order.map((item) => `build:${item}`), ...job.order.map((item) => `publish:${item}`)];
  $("#job-steps").innerHTML = names.map((name) => `<div class="step ${job.completed.includes(name) ? "done" : job.currentStep === name ? job.status === "running" ? "running" : "blocked" : ""}">${escapeHtml(name.replace(":", " · "))}</div>`).join("");
  const log = $("#job-log");
  const nearBottom = log.scrollTop + log.clientHeight >= log.scrollHeight - 35;
  log.textContent = job.logTail || "Waiting for output…";
  if (nearBottom) log.scrollTop = log.scrollHeight;
  $("#job-error").textContent = job.error || "";
  $("#job-error").classList.toggle("hidden", !job.error);
  const canResume = ["blocked", "interrupted"].includes(job.status);
  $("#resume-area").classList.toggle("hidden", !canResume);
  if (canResume) $("#resume-confirm").placeholder = `RESUME ${job.trainId}`;
  if (job.status === "complete" && snapshot?.trainId !== job.trainId) loadState();
}

async function loadJob(train) {
  try { const result = await api(`/api/job${train ? `?train=${encodeURIComponent(train)}` : ""}`); renderJob(result.job); }
  catch (error) { notice(error.message, "error"); }
}

async function loadState() {
  try {
    snapshot = await api("/api/state");
    $("#current-train").textContent = snapshot.trainId;
    const published = snapshot.components.filter((item) => item.state === "published").length;
    $("#train-published").textContent = `${published} component(s) published in this train`;
    const pill = $("#storage-pill"); pill.textContent = snapshot.storageAvailable ? "Release SSD available" : "Release SSD will mount at start";
    pill.className = `pill ${snapshot.storageAvailable ? "" : "warning"}`;
    renderComponents(); renderRepos();
    if (snapshot.job) renderJob(snapshot.job);
  } catch (error) { notice(error.message, "error"); }
}

$("#refresh").addEventListener("click", async () => { clearNotice(); await loadState(); invalidatePreview(); });
$("#release-summary").addEventListener("input", invalidatePreview);
$("#generate-summary").addEventListener("click", () => {
  const selected = selectedComponents();
  if (!selected.length) { notice("Select at least one component before generating a release summary."); return; }
  $("#release-summary").value = generatedSummary(selected);
  invalidatePreview();
});
$("#select-all-components").addEventListener("change", (event) => {
  for (const component of snapshot.components) {
    if (component.localBuild) componentDraft(component).selected = event.target.checked;
  }
  invalidatePreview(); renderComponents();
});
$("#generate-all-changes").addEventListener("click", () => {
  let generated = 0;
  for (const component of selectedComponents()) {
    const form = componentDraft(component);
    if (form.summary.trim()) continue;
    form.summary = generatedChange(component, form);
    generated++;
  }
  if (!generated) { notice("Selected components already have change notes. Use a component's Generate change button to replace one."); return; }
  invalidatePreview(); renderComponents();
  notice(`Generated ${generated} change note${generated === 1 ? "" : "s"}. Review and refine them before releasing.`, "success");
});
$("#preview-button").addEventListener("click", async () => {
  const button = $("#preview-button"); const label = button.innerHTML;
  button.disabled = true; button.textContent = "Checking source and remote branches…"; clearNotice();
  try { renderPreview(await api("/api/preview", formValue())); }
  catch (error) { notice(error.message, "error"); }
  finally { button.disabled = false; button.innerHTML = label; }
});
$("#resume-button").addEventListener("click", async () => {
  if (!currentJob) return;
  try {
    await api("/api/resume", {trainId: currentJob.trainId, confirmation: $("#resume-confirm").value});
    notice(`Resuming ${currentJob.trainId}.`, "success"); await loadJob(currentJob.trainId);
  } catch (error) { notice(error.message, "error"); }
});
loadState();
polling = setInterval(() => { if (currentJob && ["running", "blocked", "interrupted"].includes(currentJob.status)) loadJob(currentJob.trainId); }, 1800);
