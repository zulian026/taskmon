import { invoke } from "@tauri-apps/api/core";

/* ---------- Types (backend API contract) ---------- */

interface ProcessView {
  pid: number;
  name: string;
  user: string;
  cpu: number;
  mem_bytes: number;
  mem_pct: number;
  status: string;
  parent: number | null;
  run_time: number;
  disk_read: number;
  disk_write: number;
}

interface DiskView { name: string; mount: string; fs: string; total: number; available: number; }
interface NetView { name: string; rx_bps: number; tx_bps: number; rx_total: number; tx_total: number; }
interface HistoryPoint { t: number; cpu: number; mem_pct: number; net_rx: number; net_tx: number; disk_r: number; disk_w: number; }

interface Snapshot {
  timestamp_ms: number;
  cpu_total: number;
  cpu_per_core: number[];
  cpu_count: number;
  mem_total: number;
  mem_used: number;
  mem_free: number;
  swap_total: number;
  swap_used: number;
  load_avg: [number, number, number];
  uptime_secs: number;
  processes: ProcessView[];
  disks: DiskView[];
  networks: NetView[];
  disk_read_bps: number;
  disk_write_bps: number;
  history: HistoryPoint[];
}

interface ProcessDetail {
  pid: number; name: string; user: string; cpu: number; mem_bytes: number;
  status: string; parent: number | null; run_time: number; start_time: number;
  exe: string | null; cmdline: string; cwd: string | null;
  disk_read_total: number; disk_write_total: number;
}

interface StartupEntry { id: string; name: string; exec: string; comment: string | null; source: string; enabled: boolean; path: string; }
interface ServiceView { unit: string; load: string; active: string; sub: string; description: string; }

/* ---------- Formatting ---------- */

function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = n / 1024;
  let u = 0;
  while (v >= 1024 && u < units.length - 1) { v /= 1024; u++; }
  return `${v.toFixed(v >= 100 ? 0 : 1)} ${units[u]}`;
}

function fmtBps(n: number): string {
  return `${fmtBytes(n)}/s`;
}

function fmtDuration(secs: number): string {
  const s = Math.floor(secs);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${s % 60}s`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ${m % 60}m`;
  return `${Math.floor(h / 24)}d ${h % 24}h`;
}

function fmtClock(ts: number): string {
  return new Date(ts).toLocaleTimeString("en-GB", { hour12: false });
}

function fmtDateTime(unixSecs: number): string {
  return new Date(unixSecs * 1000).toLocaleString("en-GB", { hour12: false });
}

const $ = <T extends HTMLElement>(id: string): T => {
  const el = document.getElementById(id);
  if (!el) throw new Error(`missing #${id}`);
  return el as T;
};

/* ---------- State ---------- */

let snap: Snapshot | null = null;
let view: "processes" | "performance" | "startup" | "services" = "processes";
let query = "";
let sortKey: keyof ProcessView = "cpu";
let sortDir: 1 | -1 = -1;
let selectedPid: number | null = null;
let selectedName = "";
let startupEntries: StartupEntry[] = [];
let serviceScope: "user" | "system" = "user";
let serviceList: ServiceView[] = [];
let pollTimer: number | undefined;

/* ---------- Charts ---------- */

function lineChart(el: HTMLElement, points: { t: number; v: number }[], max: number): void {
  const W = 720;
  const H = 140;
  const padL = 36;
  const padB = 18;
  const padT = 8;
  const iw = W - padL - 8;
  const ih = H - padT - padB;
  const n = points.length;
  const xs = (i: number) => padL + (n <= 1 ? iw / 2 : (i / (n - 1)) * iw);
  const ys = (v: number) => padT + ih - Math.min(v, max) / max * ih;
  let grid = "";
  for (const g of [0, 25, 50, 75, 100]) {
    const y = ys(g);
    grid += `<line x1="${padL}" y1="${y}" x2="${W - 8}" y2="${y}" stroke-width="1"/><text x="${padL - 6}" y="${y + 4}" font-size="11" text-anchor="end">${g}</text>`;
  }
  const d = points.map((p, i) => `${i === 0 ? "M" : "L"}${xs(i).toFixed(1)},${ys(p.v).toFixed(1)}`).join(" ");
  el.innerHTML =
    `<svg viewBox="0 0 ${W} ${H}" preserveAspectRatio="none" role="img" aria-label="History chart">` +
    grid +
    `<path d="${d}" fill="none" stroke-width="2"/>` +
    `</svg>`;
}

/* ---------- Processes ---------- */

function visibleProcesses(): ProcessView[] {
  if (!snap) return [];
  const q = query.trim().toLowerCase();
  let rows = snap.processes;
  if (q) {
    rows = rows.filter(
      (p) =>
        p.name.toLowerCase().includes(q) ||
        p.user.toLowerCase().includes(q) ||
        String(p.pid).includes(q),
    );
  }
  const key = sortKey;
  rows = [...rows].sort((a, b) => {
    const av = a[key];
    const bv = b[key];
    if (typeof av === "number" && typeof bv === "number") return (av - bv) * sortDir;
    return String(av).localeCompare(String(bv)) * sortDir;
  });
  return rows;
}

function renderProcesses(): void {
  if (!snap) return;
  const rows = visibleProcesses();
  $("proc-count").textContent = `${rows.length} of ${snap.processes.length} processes`;
  const top = snap.processes[0];
  $("proc-top").textContent = top
    ? `top CPU: ${top.name} ${top.cpu.toFixed(1)}%`
    : "no processes";
  const body = $("proc-body");
  const frag = document.createDocumentFragment();
  for (const p of rows.slice(0, 300)) {
    const tr = document.createElement("tr");
    if (p.pid === selectedPid) tr.classList.add("selected");
    tr.dataset.pid = String(p.pid);
    tr.innerHTML =
      `<td class="name-cell" title="${escapeHtml(p.name)}">${escapeHtml(p.name)}</td>` +
      `<td class="num">${p.pid}</td>` +
      `<td>${escapeHtml(p.user)}</td>` +
      `<td class="num">${p.cpu.toFixed(1)}%</td>` +
      `<td class="num">${fmtBytes(p.mem_bytes)}</td>` +
      `<td>${escapeHtml(p.status)}</td>` +
      `<td class="num">${fmtDuration(p.run_time)}</td>`;
    tr.addEventListener("click", () => selectProcess(p.pid, p.name));
    frag.appendChild(tr);
  }
  body.replaceChildren(frag);
  if (rows.length > 300) {
    const tr = document.createElement("tr");
    tr.innerHTML = `<td colspan="7" class="meta">Showing 300 of ${rows.length} — narrow the search to see more.</td>`;
    body.appendChild(tr);
  }
  $<HTMLButtonElement>("end-task-btn").disabled = selectedPid === null;
}

function escapeHtml(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

async function selectProcess(pid: number, name: string): Promise<void> {
  selectedPid = pid;
  selectedName = name;
  renderProcesses();
  const panel = $("proc-detail");
  panel.hidden = false;
  $("detail-name").textContent = name;
  $("detail-pid").textContent = `PID ${pid}`;
  try {
    const d = await invoke<ProcessDetail>("get_process_detail", { pid });
    $("detail-cmd").textContent = d.cmdline || d.name;
    $("detail-exe").textContent = d.exe ?? "—";
    $("detail-cwd").textContent = d.cwd ?? "—";
    $("detail-parent").textContent = d.parent === null ? "—" : String(d.parent);
    $("detail-start").textContent = fmtDateTime(d.start_time);
    $("detail-disk").textContent = `${fmtBytes(d.disk_read_total)} read · ${fmtBytes(d.disk_write_total)} written`;
  } catch (e) {
    $("detail-cmd").textContent = `Could not read details: ${e}`;
  }
}

async function endTask(pid: number, name: string, force: boolean): Promise<void> {
  const label = force ? "Force kill" : "End task";
  if (!window.confirm(`${label} ${name} (PID ${pid})?${force ? "" : " The process gets SIGTERM and can shut down cleanly."}`)) return;
  const status = $("proc-status");
  try {
    await invoke("kill_process", { pid, force });
    status.textContent = `${label}: signal sent to ${name} (PID ${pid}).`;
    if (selectedPid === pid) {
      selectedPid = null;
      $("proc-detail").hidden = true;
    }
    await refreshSoon();
  } catch (e) {
    status.textContent = `${label} failed: ${e}`;
  }
}

/* ---------- Performance ---------- */

function renderPerformance(): void {
  if (!snap) return;
  const memPct = snap.mem_total ? (snap.mem_used / snap.mem_total) * 100 : 0;
  const swapPct = snap.swap_total ? (snap.swap_used / snap.swap_total) * 100 : 0;
  $("perf-sub").textContent =
    `${snap.cpu_count} cores · uptime ${fmtDuration(snap.uptime_secs)} · load ${snap.load_avg.map((v) => v.toFixed(2)).join(" ")}`;
  $("kpi-cpu").textContent = `${snap.cpu_total.toFixed(1)}%`;
  $("kpi-cpu-note").textContent = `load ${snap.load_avg[0].toFixed(2)} (1 min)`;
  $("kpi-mem").textContent = `${memPct.toFixed(1)}%`;
  $("kpi-mem-note").textContent = `${fmtBytes(snap.mem_used)} of ${fmtBytes(snap.mem_total)}`;
  $("kpi-disk").textContent = fmtBps(snap.disk_read_bps);
  $("kpi-disk-note").textContent = "read";
  $("kpi-net").textContent = fmtBps(snap.networks.filter((n) => n.name !== "lo").reduce((a, n) => a + n.rx_bps, 0));
  $("kpi-net-note").textContent = "download";

  lineChart($("cpu-chart"), snap.history.map((h) => ({ t: h.t, v: h.cpu })), 100);
  lineChart($("mem-chart"), snap.history.map((h) => ({ t: h.t, v: h.mem_pct })), 100);

  const top = $("top-list");
  top.replaceChildren(
    ...snap.processes.slice(0, 5).map((p) => {
      const li = document.createElement("li");
      li.innerHTML = `<span class="mono">${escapeHtml(p.name)}</span><span class="num">${p.cpu.toFixed(1)}% · ${fmtBytes(p.mem_bytes)}</span>`;
      return li;
    }),
  );
  $("load-avg").textContent = snap.load_avg.map((v) => v.toFixed(2)).join(" ");

  $("core-count").textContent = `${snap.cpu_count} cores`;
  const cores = $("core-bars");
  cores.replaceChildren(
    ...snap.cpu_per_core.map((c, i) => {
      const div = document.createElement("div");
      div.className = "core-bar";
      div.innerHTML = `<span>CPU ${i}</span><div class="meter"><div class="meter-fill" style="width:${Math.min(c, 100).toFixed(1)}%"></div></div><strong>${c.toFixed(0)}%</strong>`;
      return div;
    }),
  );

  $("ram-fill").style.width = `${memPct.toFixed(1)}%`;
  $("ram-text").textContent = `${fmtBytes(snap.mem_used)} / ${fmtBytes(snap.mem_total)}`;
  $("swap-fill").style.width = `${swapPct.toFixed(1)}%`;
  $("swap-text").textContent = snap.swap_total
    ? `${fmtBytes(snap.swap_used)} / ${fmtBytes(snap.swap_total)}`
    : "no swap";
  $("disk-list").innerHTML = snap.disks.length
    ? snap.disks.map((d) => {
        const used = d.total ? 100 - (d.available / d.total) * 100 : 0;
        return `<div class="kv-row"><span>${escapeHtml(d.mount)} <span class="meta">${escapeHtml(d.name)} · ${escapeHtml(d.fs)}</span></span><span class="num">${used.toFixed(0)}% used · ${fmtBytes(d.available)} free</span></div>`;
      }).join("")
    : `<p class="empty-note">No disks reported.</p>`;
  const nets = snap.networks.filter((n) => n.name !== "lo");
  $("net-list").innerHTML = nets.length
    ? nets.map((n) => `<div class="kv-row"><span>${escapeHtml(n.name)}</span><span class="num">↓ ${fmtBps(n.rx_bps)} · ↑ ${fmtBps(n.tx_bps)}</span></div>`).join("")
    : `<p class="empty-note">No network interfaces reported.</p>`;
}

/* ---------- Startup ---------- */

async function loadStartup(): Promise<void> {
  try {
    startupEntries = await invoke<StartupEntry[]>("get_startup");
  } catch (e) {
    $("startup-status").textContent = `Could not list startup apps: ${e}`;
    return;
  }
  renderStartup();
}

function renderStartup(): void {
  const q = query.trim().toLowerCase();
  const list = $("startup-list");
  const rows = startupEntries.filter(
    (e) => !q || e.name.toLowerCase().includes(q) || e.exec.toLowerCase().includes(q),
  );
  if (!rows.length) {
    list.innerHTML = `<p class="empty-note">No startup apps found.</p>`;
    return;
  }
  list.replaceChildren(
    ...rows.map((e) => {
      const div = document.createElement("div");
      div.className = "row-item";
      div.innerHTML =
        `<div class="row-main"><div class="row-name">${escapeHtml(e.name)}</div>` +
        `<div class="row-sub">${escapeHtml(e.exec)}</div></div>` +
        `<span class="row-source">${e.source}</span>`;
      const toggle = document.createElement("button");
      toggle.type = "button";
      toggle.className = "toggle";
      toggle.setAttribute("role", "switch");
      toggle.setAttribute("aria-checked", String(e.enabled));
      toggle.setAttribute("aria-label", `Enable ${e.name} at login`);
      toggle.addEventListener("click", async () => {
        toggle.disabled = true;
        try {
          const updated = await invoke<StartupEntry>("set_startup", { id: e.id, enabled: !e.enabled });
          startupEntries = startupEntries.map((x) => (x.id === e.id ? updated : x));
          $("startup-status").textContent = `${updated.name} ${updated.enabled ? "will start at login" : "will no longer start at login"}.`;
          renderStartup();
        } catch (err) {
          $("startup-status").textContent = `Could not change ${e.name}: ${err}`;
          toggle.disabled = false;
        }
      });
      div.appendChild(toggle);
      return div;
    }),
  );
}

/* ---------- Services ---------- */

async function loadServices(): Promise<void> {
  try {
    serviceList = await invoke<ServiceView[]>("get_services", { scope: serviceScope });
    $("svc-status").textContent = `${serviceList.length} services (${serviceScope}).`;
  } catch (e) {
    $("svc-status").textContent = `Could not list services: ${e}`;
    serviceList = [];
  }
  renderServices();
}

function renderServices(): void {
  const q = query.trim().toLowerCase();
  const rows = serviceList.filter(
    (s) =>
      !q ||
      s.unit.toLowerCase().includes(q) ||
      s.description.toLowerCase().includes(q) ||
      s.active.toLowerCase().includes(q),
  );
  const body = $("svc-body");
  body.replaceChildren(
    ...rows.slice(0, 200).map((s) => {
      const tr = document.createElement("tr");
      const running = s.active === "active";
      tr.innerHTML =
        `<td class="name-cell" title="${escapeHtml(s.unit)}">${escapeHtml(s.unit)}</td>` +
        `<td>${escapeHtml(s.load)}</td>` +
        `<td>${escapeHtml(s.active)}</td>` +
        `<td>${escapeHtml(s.sub)}</td>` +
        `<td class="desc-cell" title="${escapeHtml(s.description || "")}">${escapeHtml(s.description || "—")}</td>` +
        `<td class="num"><span class="row-actions"></span></td>`;
      const actions = tr.querySelector(".row-actions") as HTMLElement;
      const actionIcons: Record<string, "play" | "stop" | "refresh"> = { start: "play", stop: "stop", restart: "refresh" };
      for (const action of running ? ["stop", "restart"] : ["start"]) {
        const btn = document.createElement("button");
        btn.type = "button";
        btn.className = "btn btn-sm btn-outline";
        btn.innerHTML = `${icon(actionIcons[action])}${action}`;
        btn.addEventListener("click", async (ev) => {
          ev.stopPropagation();
          btn.disabled = true;
          try {
            const r = await invoke<{ ok: boolean; stdout: string; stderr: string }>("service_action", {
              scope: serviceScope,
              unit: s.unit,
              action,
            });
            $("svc-status").textContent = r.ok
              ? `${action} ${s.unit}: done.`
              : `${action} ${s.unit} failed: ${r.stderr || r.stdout || "unknown error"}`;
            await loadServices();
          } catch (err) {
            $("svc-status").textContent = `${action} ${s.unit} failed: ${err}`;
            btn.disabled = false;
          }
        });
        actions.appendChild(btn);
      }
      return tr;
    }),
  );
}

/* ---------- Icons ---------- */

/* Same stroke set as the static SVGs in index.html: 16px, 1.5 stroke,
   currentColor. Kept in one map so dynamic buttons match. */
const ICONS: Record<string, string> = {
  sun: '<circle cx="8" cy="8" r="3.2"/><path d="M8 1.5v1.8M8 12.7v1.8M1.5 8h1.8M12.7 8h1.8M3.4 3.4l1.3 1.3M11.3 11.3l1.3 1.3M12.6 3.4l-1.3 1.3M4.7 11.3l-1.3 1.3"/>',
  moon: '<path d="M13.5 9.8A5.8 5.8 0 1 1 6.2 2.5a4.6 4.6 0 0 0 7.3 7.3z"/>',
  play: '<path d="M5 3.5l8 4.5-8 4.5z" fill="currentColor" stroke="none"/>',
  stop: '<rect x="4" y="4" width="8" height="8" rx="1" fill="currentColor" stroke="none"/>',
  refresh: '<path d="M13.5 8a5.5 5.5 0 1 1-1.6-3.9"/><path d="M13.7 1.2v3.2h-3.2"/>',
};

function icon(name: keyof typeof ICONS): string {
  return `<svg class="icon" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${ICONS[name]}</svg>`;
}

/* ---------- Theme ---------- */

function currentTheme(): "light" | "dark" {
  return document.documentElement.dataset.theme === "light" ? "light" : "dark";
}

function applyTheme(t: "light" | "dark"): void {
  document.documentElement.dataset.theme = t;
  try {
    localStorage.setItem("taskmon-theme", t);
  } catch {
    /* private mode — theme just doesn't persist */
  }
  $("theme-toggle").innerHTML = t === "light" ? `${icon("moon")}Dark mode` : `${icon("sun")}Light mode`;
}

function initTheme(): void {
  applyTheme(currentTheme());
  $("theme-toggle").addEventListener("click", () => {
    applyTheme(currentTheme() === "light" ? "dark" : "light");
  });
}

/* ---------- Shell ---------- */

function switchView(next: typeof view): void {
  view = next;
  document.querySelectorAll(".nav-item").forEach((b) => {
    b.classList.toggle("is-active", (b as HTMLElement).dataset.view === next);
  });
  document.querySelectorAll(".view").forEach((v) => {
    (v as HTMLElement).hidden = v.id !== `view-${next}`;
  });
  if (next === "startup") void loadStartup();
  if (next === "services") void loadServices();
  if (next === "processes") renderProcesses();
  if (next === "performance") renderPerformance();
}

function renderSidebar(): void {
  if (!snap) return;
  $("side-cpu").textContent = `${snap.cpu_total.toFixed(1)}%`;
  const memPct = snap.mem_total ? (snap.mem_used / snap.mem_total) * 100 : 0;
  $("side-mem").textContent = `${memPct.toFixed(0)}%`;
  $("side-uptime").textContent = fmtDuration(snap.uptime_secs);
}

async function poll(): Promise<void> {
  try {
    snap = await invoke<Snapshot>("get_snapshot");
    $("freshness").textContent = `Updated ${fmtClock(snap.timestamp_ms)}`;
    renderSidebar();
    if (view === "processes") renderProcesses();
    if (view === "performance") renderPerformance();
  } catch (e) {
    $("freshness").textContent = `Snapshot failed: ${e}`;
  }
  const interval = Number(($("interval") as HTMLSelectElement).value);
  pollTimer = window.setTimeout(() => void poll(), interval);
}

async function refreshSoon(): Promise<void> {
  window.clearTimeout(pollTimer);
  await poll();
}

function init(): void {
  initTheme();
  document.querySelectorAll(".nav-item").forEach((b) => {
    b.addEventListener("click", () => switchView((b as HTMLElement).dataset.view as typeof view));
  });
  $("top-search").addEventListener("input", (ev) => {
    query = (ev.target as HTMLInputElement).value;
    if (view === "processes") renderProcesses();
    if (view === "startup") renderStartup();
    if (view === "services") renderServices();
  });
  $("interval").addEventListener("change", () => void refreshSoon());
  document.querySelectorAll("#proc-table th[data-sort]").forEach((th) => {
    th.addEventListener("click", () => {
      const key = (th as HTMLElement).dataset.sort as keyof ProcessView;
      if (sortKey === key) {
        sortDir = sortDir === -1 ? 1 : -1;
      } else {
        sortKey = key;
        sortDir = key === "name" || key === "user" || key === "status" ? 1 : -1;
      }
      document.querySelectorAll("#proc-table th").forEach((h) => h.classList.remove("sorted-asc", "sorted-desc"));
      th.classList.add(sortDir === 1 ? "sorted-asc" : "sorted-desc");
      renderProcesses();
    });
  });
  $("end-task-btn").addEventListener("click", () => {
    if (selectedPid !== null) void endTask(selectedPid, selectedName, false);
  });
  $("detail-end").addEventListener("click", () => {
    if (selectedPid !== null) void endTask(selectedPid, selectedName, false);
  });
  $("detail-force").addEventListener("click", () => {
    if (selectedPid !== null) void endTask(selectedPid, selectedName, true);
  });
  $("startup-refresh").addEventListener("click", () => void loadStartup());
  $("scope-user").addEventListener("click", () => {
    serviceScope = "user";
    $("scope-user").classList.add("is-active");
    $("scope-system").classList.remove("is-active");
    void loadServices();
  });
  $("scope-system").addEventListener("click", () => {
    serviceScope = "system";
    $("scope-system").classList.add("is-active");
    $("scope-user").classList.remove("is-active");
    void loadServices();
  });
  void poll();
}

init();
