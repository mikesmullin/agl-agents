// email-trainer/public/js/app.js
// Alpine.js application for email triage UI

// =============================================================================
// Rainbow color interpolation (port of d3.interpolateRainbow)
// =============================================================================
function interpolateRainbow(t) {
  t = t - Math.floor(t);
  const ts = Math.abs(t - 0.5);
  const h = 360 * t - 100;
  const s = 1.5 - 1.5 * ts;
  const l = 0.8 - 0.9 * ts;
  const hRad = h * Math.PI / 180;
  const x = Math.pow(l, 3) * (s * Math.cos(hRad) + 1);
  const y = Math.pow(l, 3) * s * Math.sin(hRad);
  const z = Math.pow(l, 3);
  let r = 0.787 * x - 0.213 * y;
  let g = -0.393 * x + 0.715 * y - 0.072 * z;
  let b = -0.072 * y + 1.000 * z;
  r = Math.max(0, Math.min(255, Math.round(255 * r)));
  g = Math.max(0, Math.min(255, Math.round(255 * g)));
  b = Math.max(0, Math.min(255, Math.round(255 * b)));
  return `rgb(${r}, ${g}, ${b})`;
}

// =============================================================================
// Pipeline stage definitions
// =============================================================================
const STAGES = [
  { id: 'new', label: 'New', emoji: '📬', t: 0.05, human: false },
  { id: 'loading', label: 'Loading', emoji: '📥', t: 0.14, human: false },
  { id: 'analyzing', label: 'Analyzing', emoji: '🔍', t: 0.26, human: false },
  { id: 'recommended', label: 'Recommended', emoji: '💡', t: 0.38, human: false },
  { id: 'awaiting_input', label: 'Awaiting Input', emoji: '✏️', t: 0.47, human: true },
  { id: 'processing', label: 'Processing', emoji: '⚙️', t: 0.58, human: false },
  { id: 'awaiting_approval', label: 'Awaiting Approval', emoji: '☑️', t: 0.68, human: true },
  { id: 'applying', label: 'Applying', emoji: '🚀', t: 0.79, human: false },
  { id: 'done', label: 'Done', emoji: '🎉', t: 0.91, human: false },
  { id: 'skipped', label: 'Skipped', emoji: '⏭️', t: 0.97, human: false },
];

const STAGE_MAP = Object.fromEntries(STAGES.map(s => [s.id, s]));

function getEntityStage(e) {
  if (!e) return STAGE_MAP['new'];
  if (e.skip?.active) return STAGE_MAP['skipped'];
  if (e.apply?.applied_at) return STAGE_MAP['done'];
  if (e.apply?.approved === true && !e.apply?.applied_at) return STAGE_MAP['applying'];
  if (e.plan && (e.apply == null || e.apply?.approved == null)) return STAGE_MAP['awaiting_approval'];
  // operator_input processed but still executing/journaling/planning
  if ((e.operator_input?.processed) || e.execution || e.journal) return STAGE_MAP['processing'];
  // operator_input filled by human but not yet processed by agent
  if (e.operator_input && e.operator_input.instruction !== null && e.operator_input.instruction !== undefined) {
    return STAGE_MAP['processing'];
  }
  // operator_input gate is open (instruction === null) — human action required
  if (e.operator_input && e.operator_input.instruction === null) return STAGE_MAP['awaiting_input'];
  // pipeline running
  if (e.recommendation) return STAGE_MAP['recommended'];
  if (e.recall || e.summary || e.fingerprint) return STAGE_MAP['analyzing'];
  if (e.content?.body) return STAGE_MAP['loading'];
  return STAGE_MAP['new'];
}

// =============================================================================
// Utilities
// =============================================================================
function stripAnsi(str) {
  if (!str) return '';
  return String(str).replace(/\x1b\[[0-9;]*[mGKJHF]/g, '').replace(/\\e\[[0-9;]*[mGKJHF]/g, '');
}

function formatDate(iso) {
  if (!iso) return '';
  try {
    const d = new Date(iso);
    return d.toLocaleDateString('en-US', {
      month: 'short', day: 'numeric', year: 'numeric',
      hour: '2-digit', minute: '2-digit',
    });
  } catch { return iso; }
}

function triggerConfetti() {
  if (typeof confetti === 'undefined') return;
  confetti({
    particleCount: 130,
    spread: 80,
    origin: { y: 0.65 },
    zIndex: 9000,
    disableForReducedMotion: true,
  });
  // Second burst slightly offset
  setTimeout(() => confetti({
    particleCount: 60,
    spread: 100,
    origin: { x: 0.3, y: 0.6 },
    zIndex: 9000,
    disableForReducedMotion: true,
  }), 180);
}

// =============================================================================
// Main Alpine.js app
// =============================================================================
function app() {
  return {
    entities: [],
    filter: null,          // stage id or null (show all)
    ws: null,
    wsStatus: 'connecting',
    forms: {},             // keyed by entity.id — form + UI state per entity
    showAllStages: false,  // sidebar: show all stages vs. only stages with counts
    cfgDestinations: [],   // populated from /api/config
    focusedId: null,       // entity ID of the focused card (stable across list reorders)
    slots: Array.from({ length: 10 }, () => ({ entityId: null })),  // fixed 10-slot display window

    // Training section state
    view: 'triage',        // 'triage' | 'operator-history' | 'trial-history' | 'trial-run'
    archiveEntries: [],    // loaded from /api/archive
    archiveLoading: false,
    archiveSortCol: 'apply.applied_at',
    archiveSortDir: -1,    // -1 = desc, 1 = asc
    archiveCopied: null,   // entity id that was just copied
    archiveSearch: '',     // keyword search box
    archiveFilterInstruction: '', // operator_input._parsed_operation dropdown filter
    archiveDeleteId: null, // entity id pending delete confirmation

    // Trial History state
    trialEntries: [],
    trialLoading: false,
    trialSortCol: 'id',
    trialSortDir: -1,
    trialCopied: null,
    trialDeleteId: null,
    trialPromoteId: null,
    trialPromoting: false,
    trialPromoteResult: null, // { ok, output, error } after last promote
    trialChartMetric: 'passing', // which column is plotted in the bar chart

    // Trial Run state
    trialRunStatus: { running: false, lock: null, progress: null },
    trialRunLoading: false,
    trialRunStarting: false,
    trialRunStopping: false,
    trialRunPollInterval: null,
    trialRunFromId: '',
    trialRunError: null,

    // -------------------------------------------------------------------------
    // Computed
    // -------------------------------------------------------------------------
    get filteredEntities() {
      // Sort by email date ascending (oldest first) — stable position regardless of stage transitions
      const sorted = [...this.entities].sort((a, b) => {
        const da = a.envelope?.date ? new Date(a.envelope.date).getTime() : 0;
        const db = b.envelope?.date ? new Date(b.envelope.date).getTime() : 0;
        return da - db;
      });
      if (!this.filter) return sorted;
      return sorted.filter(e => getEntityStage(e).id === this.filter);
    },

    get focusedIdx() {
      if (!this.focusedId) return 0;
      const idx = this.filteredEntities.findIndex(e => e.id === this.focusedId);
      return idx >= 0 ? idx : 0;
    },

    get focusedStageId() {
      const ent = this.getEntityById(this.focusedId);
      return ent ? getEntityStage(ent).id : null;
    },

    get stageCounts() {
      const counts = {};
      STAGES.forEach(s => counts[s.id] = 0);
      this.entities.forEach(e => {
        const stage = getEntityStage(e);
        counts[stage.id] = (counts[stage.id] || 0) + 1;
      });

      return counts;
    },

    get stages() { return STAGES; },

    get visibleStages() {
      return STAGES.filter(s => this.stageCounts[s.id] > 0 || this.showAllStages);
    },

    get destinations() {
      return this.cfgDestinations.length ? this.cfgDestinations : [
        'Expenses', 'Statements', 'Newsletters', 'Opportunities', 'Travel',
        'Stock', 'Taxes', 'Kids', 'Job Applications', 'Job Interviews',
      ];
    },

    // -------------------------------------------------------------------------
    // Init
    // -------------------------------------------------------------------------
    init() {
      this.connectWS();
      this.loadConfig();
      document.addEventListener('keydown', e => this.handleHotkey(e));
      // Reset slots when filter changes, then backfill from new filtered list
      this.$watch('filter', () => {
        this.slots = Array.from({ length: 10 }, () => ({ entityId: null }));
        this.$nextTick(() => this._syncSlots());
      });
      this.$nextTick(() => this._syncSlots());
    },

    async loadConfig() {
      try {
        const res = await fetch('/api/config');
        const cfg = await res.json();
        if (cfg.destinations?.length) this.cfgDestinations = cfg.destinations;
      } catch { }
    },

    async loadArchive() {
      this.archiveLoading = true;
      try {
        const res = await fetch('/api/archive');
        this.archiveEntries = await res.json();
      } catch { this.archiveEntries = []; }
      this.archiveLoading = false;
    },

    get filteredArchiveEntries() {
      let entries = this.archiveEntries;
      // Keyword search across row values
      const q = this.archiveSearch.trim().toLowerCase();
      if (q) {
        entries = entries.filter(e => [
          e.id, e.envelope?.from, e.envelope?.date,
          e.summary?.headline, e.summary?.description,
          e.execution?.instruction, e.operator_input?.instruction,
          e.recommendation?.operations, e.recommendation?.confidence,
          e.recommendation?.journal_id, e.journal_meta?.confirmed_count,
        ].some(v => v != null && String(v).toLowerCase().includes(q)));
      }
      // _parsed_operation dropdown filter
      if (this.archiveFilterInstruction) {
        entries = entries.filter(e =>
          (e.operator_input?._parsed_operation ?? '') === this.archiveFilterInstruction
        );
      }
      return entries;
    },

    get sortedArchiveEntries() {
      const col = this.archiveSortCol;
      const dir = this.archiveSortDir;
      return [...this.filteredArchiveEntries].sort((a, b) => {
        const av = this._archiveVal(a, col);
        const bv = this._archiveVal(b, col);
        if (av < bv) return -dir;
        if (av > bv) return dir;
        return 0;
      });
    },

    get archiveInstructionOptions() {
      const set = new Set();
      this.archiveEntries.forEach(e => {
        const v = e.operator_input?._parsed_operation;
        if (v) set.add(v);
      });
      return [...set].sort();
    },

    get trialChartEntries() {
      const metric = this.trialChartMetric;
      return [...this.trialEntries]
        .filter(t => this._trialChartVal(t, metric) != null)
        .sort((a, b) => String(a.id).localeCompare(String(b.id)));
    },

    get maxTrialChartVal() {
      const metric = this.trialChartMetric;
      const vals = this.trialChartEntries.map(t => this._trialChartVal(t, metric));
      return vals.length ? Math.max(...vals) : 1;
    },

    get trialChartLabel() {
      const labels = { passing: 'Passing', total: 'Total', score: 'Score (%)', grade: 'Grade', duration: 'Duration (s)' };
      return labels[this.trialChartMetric] ?? this.trialChartMetric;
    },

    _trialChartVal(trial, metric) {
      switch (metric) {
        case 'passing': return trial.passing;
        case 'total': return trial.total;
        case 'score': return trial.score;
        case 'grade': {
          if (trial.grade == null) return null;
          const map = { 'A+': 13, A: 12, 'A-': 11, 'B+': 10, B: 9, 'B-': 8, 'C+': 7, C: 6, 'C-': 5, 'D+': 4, D: 3, 'D-': 2, F: 1 };
          return map[trial.grade] ?? null;
        }
        case 'duration': return trial.duration_ms != null ? Math.round(trial.duration_ms / 1000) : null;
        default: return null;
      }
    },

    trialChartTooltip(trial) {
      const metric = this.trialChartMetric;
      const v = this._trialChartVal(trial, metric);
      if (metric === 'duration') return `${trial.id}: ${this.durationRelative(trial.duration_ms)}`;
      if (metric === 'grade') return `${trial.id}: ${trial.grade} (${v})`;
      if (metric === 'score') return `${trial.id}: ${v}%`;
      return `${trial.id}: ${v ?? '—'}`;
    },

    durationRelative(ms) {
      if (ms == null) return '—';
      const s = Math.round(ms / 1000);
      if (s < 60) return `${s}s`;
      const m = Math.floor(s / 60), rs = s % 60;
      if (m < 60) return rs ? `${m}m ${rs}s` : `${m}m`;
      const h = Math.floor(m / 60), rm = m % 60;
      return rm ? `${h}h ${rm}m` : `${h}h`;
    },

    setTrialChartMetric(metric) { this.trialChartMetric = metric; },

    get sortedTrialEntries() {
      const col = this.trialSortCol;
      const dir = this.trialSortDir;
      return [...this.trialEntries].sort((a, b) => {
        let av, bv;
        switch (col) {
          case 'id': av = String(a.id ?? ''); bv = String(b.id ?? ''); break;
          case 'date': av = a.date ? new Date(a.date).getTime() : 0; bv = b.date ? new Date(b.date).getTime() : 0; break;
          case 'passing': av = a.passing ?? 0; bv = b.passing ?? 0; break;
          case 'total': av = a.total ?? 0; bv = b.total ?? 0; break;
          case 'score': av = a.score ?? 0; bv = b.score ?? 0; break;
          case 'grade': av = String(a.grade ?? ''); bv = String(b.grade ?? ''); break;
          case 'duration': av = a.duration_ms ?? -1; bv = b.duration_ms ?? -1; break;
          default: av = ''; bv = '';
        }
        if (av < bv) return -dir;
        if (av > bv) return dir;
        return 0;
      });
    },

    trialSortBy(col) {
      if (this.trialSortCol === col) {
        this.trialSortDir = -this.trialSortDir;
      } else {
        this.trialSortCol = col;
        this.trialSortDir = -1;
      }
    },

    trialSortIcon(col) {
      if (this.trialSortCol !== col) return '↕';
      return this.trialSortDir === -1 ? '↓' : '↑';
    },

    trialGradeColor(grade) {
      if (!grade) return 'text-gray-500';
      const g = grade[0];
      if (g === 'A') return 'text-green-400';
      if (g === 'B') return 'text-blue-400';
      if (g === 'C') return 'text-yellow-400';
      if (g === 'D') return 'text-orange-400';
      return 'text-red-400';
    },

    async loadTrials() {
      this.trialLoading = true;
      try {
        const res = await fetch('/api/trials');
        this.trialEntries = res.ok ? await res.json() : [];
      } catch { this.trialEntries = []; }
      this.trialLoading = false;
    },

    copyTrialId(id) {
      navigator.clipboard.writeText(id).then(() => {
        this.trialCopied = id;
        setTimeout(() => { if (this.trialCopied === id) this.trialCopied = null; }, 1500);
      });
    },

    confirmTrialDelete(id) { this.trialDeleteId = id; },
    cancelTrialDelete() { this.trialDeleteId = null; },

    async deleteTrialEntry() {
      const id = this.trialDeleteId;
      if (!id) return;
      this.trialDeleteId = null;
      try {
        const res = await fetch(`/api/trials/${encodeURIComponent(id)}`, { method: 'DELETE' });
        if (res.ok) this.trialEntries = this.trialEntries.filter(e => e.id !== id);
      } catch { }
    },

    confirmTrialPromote(id) { this.trialPromoteId = id; this.trialPromoteResult = null; },
    cancelTrialPromote() { this.trialPromoteId = null; this.trialPromoteResult = null; },

    async promoteTrialEntry() {
      const id = this.trialPromoteId;
      if (!id) return;
      this.trialPromoting = true;
      this.trialPromoteResult = null;
      try {
        const res = await fetch(`/api/trials/${encodeURIComponent(id)}/promote`, { method: 'POST' });
        const data = await res.json();
        this.trialPromoteResult = res.ok ? { ok: true, output: data.output } : { ok: false, error: data.error };
      } catch (e) {
        this.trialPromoteResult = { ok: false, error: e.message };
      }
      this.trialPromoting = false;
      if (this.trialPromoteResult?.ok) {
        // auto-close after success
        setTimeout(() => { this.trialPromoteId = null; this.trialPromoteResult = null; }, 2000);
      }
    },

    // -------------------------------------------------------------------------
    // Trial Run methods
    // -------------------------------------------------------------------------
    async loadTrialRunStatus() {
      this.trialRunLoading = true;
      try {
        const res = await fetch('/api/trial-run/status');
        if (res.ok) this.trialRunStatus = await res.json();
      } catch { }
      this.trialRunLoading = false;
    },

    async startTrialRun() {
      this.trialRunStarting = true;
      this.trialRunError = null;
      try {
        const body = this.trialRunFromId.trim() ? { fromId: this.trialRunFromId.trim() } : {};
        const res = await fetch('/api/trial-run/start', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(body)
        });
        const data = await res.json();
        if (!res.ok) {
          this.trialRunError = data.error || `Error ${res.status}`;
          this.trialRunStarting = false;
        } else {
          // Poll until status confirms running, then clear the starting flag.
          // Button stays disabled (trialRunStarting=true) the whole time.
          const poll = async (attempts) => {
            await this.loadTrialRunStatus();
            if (this.trialRunStatus.running) {
              this.trialRunStarting = false;
            } else if (attempts > 0) {
              setTimeout(() => poll(attempts - 1), 1000);
            } else {
              this.trialRunStarting = false; // give up after ~6s
            }
          };
          setTimeout(() => poll(5), 1000);
        }
      } catch (e) {
        this.trialRunError = e.message;
        this.trialRunStarting = false;
      }
    },

    async stopTrialRun() {
      this.trialRunStopping = true;
      this.trialRunError = null;
      try {
        const res = await fetch('/api/trial-run/stop', { method: 'POST' });
        const data = await res.json();
        if (!res.ok) this.trialRunError = data.error || `Error ${res.status}`;
        else setTimeout(() => this.loadTrialRunStatus(), 1000);
      } catch (e) { this.trialRunError = e.message; }
      this.trialRunStopping = false;
    },

    trialRunElapsed() {
      const s = this.trialRunStatus?.lock?.started_at;
      if (!s) return '—';
      return this.durationRelative(Date.now() - new Date(s).getTime());
    },

    trialRunEta() {
      const eta = this.trialRunStatus?.progress?.timing?.eta_iso;
      if (!eta) return '—';
      const remainingMs = new Date(eta).getTime() - Date.now();
      if (remainingMs <= 0) return 'any moment';
      return 'in ' + this.durationRelative(remainingMs);
    },

    trialRunMetric(path) {
      const parts = path.split('.');
      let v = this.trialRunStatus?.progress;
      for (const p of parts) { if (v == null) return '—'; v = v[p]; }
      return v ?? '—';
    },

    _archiveVal(entry, col) {
      switch (col) {
        case 'id': return String(entry.id ?? '');
        case 'envelope.from': return String(entry.envelope?.from ?? '');
        case 'apply.applied_at': return entry.apply?.applied_at ? new Date(entry.apply.applied_at).getTime() : 0;
        case 'summary.headline': return String(entry.summary?.headline ?? '');
        case 'summary.description': return String(entry.summary?.description ?? '');
        case 'recommendation.journal_id': return parseFloat(entry.recommendation?.journal_id ?? -1);
        case 'journal_meta.confirmed_count': return parseFloat(entry.journal_meta?.confirmed_count ?? -1);
        case 'execution.instruction': return String(entry.execution?.instruction ?? '');
        case 'recommendation.operations': return String(entry.recommendation?.operations ?? '');
        case 'recommendation.confidence': return parseFloat(entry.recommendation?.confidence ?? 0);
        default: return '';
      }
    },

    archiveConfirmTooltip(entry) {
      const ts = entry.journal_meta?.last_confirmed_ts;
      if (!ts) return 'Never confirmed';
      const abs = new Date(ts).toLocaleString();
      const rel = this.relativeTime(ts);
      return `Last confirmed: ${abs} (${rel})`;
    },

    archiveSortBy(col) {
      if (this.archiveSortCol === col) {
        this.archiveSortDir = -this.archiveSortDir;
      } else {
        this.archiveSortCol = col;
        this.archiveSortDir = -1;
      }
    },

    archiveSortIcon(col) {
      if (this.archiveSortCol !== col) return '↕';
      return this.archiveSortDir === -1 ? '↓' : '↑';
    },

    setView(v) {
      this.view = v;
      if (v === 'operator-history' && this.archiveEntries.length === 0) this.loadArchive();
      if (v === 'trial-history' && this.trialEntries.length === 0) this.loadTrials();
      if (v === 'trial-run') {
        this.loadTrialRunStatus();
        if (!this.trialRunPollInterval) {
          this.trialRunPollInterval = setInterval(() => {
            if (this.view === 'trial-run') {
              this.loadTrialRunStatus();
            } else {
              clearInterval(this.trialRunPollInterval);
              this.trialRunPollInterval = null;
            }
          }, 5000);
        }
      } else if (this.trialRunPollInterval && v !== 'trial-run') {
        clearInterval(this.trialRunPollInterval);
        this.trialRunPollInterval = null;
      }
    },

    relativeTime(iso) {
      if (!iso) return '—';
      const diff = Date.now() - new Date(iso).getTime();
      const s = Math.round(diff / 1000);
      if (s < 60) return `${s}s ago`;
      const m = Math.round(s / 60);
      if (m < 60) return `${m}m ago`;
      const h = Math.round(m / 60);
      if (h < 24) return `${h}h ago`;
      const d = Math.round(h / 24);
      if (d < 30) return `${d}d ago`;
      const mo = Math.round(d / 30);
      if (mo < 12) return `${mo}mo ago`;
      return `${Math.round(mo / 12)}y ago`;
    },

    copyArchiveId(id) {
      navigator.clipboard.writeText(id).then(() => {
        this.archiveCopied = id;
        setTimeout(() => { if (this.archiveCopied === id) this.archiveCopied = null; }, 1500);
      });
    },

    confirmArchiveDelete(id) { this.archiveDeleteId = id; },
    cancelArchiveDelete() { this.archiveDeleteId = null; },

    async deleteArchiveEntry() {
      const id = this.archiveDeleteId;
      if (!id) return;
      this.archiveDeleteId = null;
      try {
        const res = await fetch(`/api/archive/${encodeURIComponent(id)}`, { method: 'DELETE' });
        if (res.ok) {
          this.archiveEntries = this.archiveEntries.filter(e => e.id !== id);
        }
      } catch { /* network error — silently ignore */ }
    },

    // -------------------------------------------------------------------------
    // Sidebar helpers
    // -------------------------------------------------------------------------
    clickStageFilter(stageId) {
      this.filter = this.filter === stageId ? null : stageId;
      this.focusedId = null;
    },

    clickEyeToggle() {
      this.showAllStages = !this.showAllStages;
    },

    // -------------------------------------------------------------------------
    // WebSocket
    // -------------------------------------------------------------------------
    connectWS() {
      const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
      const url = `${proto}//${location.host}/ws`;
      this.wsStatus = 'connecting';
      try {
        this.ws = new WebSocket(url);
        this.ws.onopen = () => { this.wsStatus = 'connected'; };
        this.ws.onclose = () => {
          this.wsStatus = 'disconnected';
          setTimeout(() => this.connectWS(), 3000);
        };
        this.ws.onerror = () => { this.wsStatus = 'disconnected'; };
        this.ws.onmessage = (evt) => {
          try { this.handleWSMessage(JSON.parse(evt.data)); } catch { }
        };
      } catch { }
    },

    handleWSMessage(msg) {
      switch (msg.type) {
        case 'init':
          this.entities = msg.entities || [];
          this.entities.forEach(e => this._ensureForm(e));
          break;

        case 'entity:new': {
          const exists = this.entities.find(e => e.id === msg.entity.id);
          if (!exists) {
            msg.entity._isNew = true;
            this.entities.push(msg.entity);
            this._ensureForm(msg.entity);
            setTimeout(() => {
              const e = this.entities.find(x => x.id === msg.entity.id);
              if (e) delete e._isNew;
            }, 800);
          }
          break;
        }

        case 'entity:modified': {
          const idx = this.entities.findIndex(e => e.id === msg.entity.id);
          if (idx >= 0) {
            this.entities[idx] = msg.entity;
          } else {
            this.entities.push(msg.entity);
            this._ensureForm(msg.entity);
          }
          break;
        }

        case 'entity:deleted': {
          const idx = this.entities.findIndex(e => e.id === msg.id);
          if (idx >= 0) {
            this.entities[idx]._deleting = true;
            setTimeout(() => {
              const i = this.entities.findIndex(e => e.id === msg.id);
              if (i >= 0) this.entities.splice(i, 1);
              delete this.forms[msg.id];
            }, 420);
          }
          break;
        }
      }
      this._syncSlots();
    },

    // -------------------------------------------------------------------------
    // Form management
    // -------------------------------------------------------------------------
    _ensureForm(entity) {
      if (this.forms[entity.id]) return;
      this.forms[entity.id] = {
        instruction: '',
        rationale: entity.operator_input?.rationale || '',
        notice_capture: entity.operator_input?.notice_capture || '',
        notice_display: entity.operator_input?.notice_display || '',
        destination: '',
        showMove: false,
        showNotice: false,
        error: null,
        submitting: false,
        submitted: false,
        approving: false,
      };
    },

    getForm(entityId) {
      if (!this.forms[entityId]) {
        const entity = this.entities.find(e => e.id === entityId);
        if (entity) this._ensureForm(entity);
        else this.forms[entityId] = {
          instruction: '', rationale: '', notice_capture: '', notice_display: '',
          destination: '', showMove: false, showNotice: false,
          error: null, submitting: false, submitted: false,
        };
      }
      return this.forms[entityId];
    },

    // -------------------------------------------------------------------------
    // Triage actions
    // -------------------------------------------------------------------------
    async submitTriage(entityId, overrideInstruction) {
      const form = this.getForm(entityId);
      if (form.submitted || form.submitting) return;  // already sent, ignore duplicate calls
      const instruction = overrideInstruction != null ? overrideInstruction : form.instruction;

      if (!instruction || !instruction.trim()) {
        form.error = '⚠️ Instruction is required';
        setTimeout(() => { form.error = null; }, 3000);
        return;
      }

      form.error = null;
      form.submitting = true;

      try {
        const patch = {
          operator_input: {
            instruction: instruction.trim(),
            rationale: form.rationale.trim() || null,
            notice_capture: form.notice_capture.trim() || null,
            notice_display: form.notice_display.trim() || null,
          },
        };

        const res = await fetch(`/api/entities/${entityId}`, {
          method: 'PATCH',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(patch),
        });

        if (!res.ok) {
          const err = await res.json().catch(() => ({}));
          form.error = err.error || `Server error ${res.status}`;
          return;
        }

        form.submitted = true;
        if (instruction === 'proceed' || instruction === 'p') triggerConfetti();

        // Advance focus to next card
        this._focusNext(entityId);

        // Do NOT reset submitted — keep it true until the entity transitions stage.
        // The WS broadcast will update the entity; resetting here causes a brief form re-flash.
      } catch (e) {
        form.error = e.message;
        form.submitting = false;  // re-enable on failure only
      }
    },

    quickAction(entityId, action) {
      const form = this.getForm(entityId);
      form.instruction = action;
      if (action === 'proceed') {
        this.submitTriage(entityId);
      } else {
        this._focusRationale(entityId);
      }
    },

    moveAction(entityId) {
      const form = this.getForm(entityId);
      if (!form.destination) {
        form.error = '⚠️ Choose a destination folder first';
        setTimeout(() => { form.error = null; }, 3000);
        return;
      }
      form.instruction = `move to ${form.destination}`;
      this._focusRationale(entityId);
    },

    _focusRationale(entityId) {
      this.$nextTick(() => {
        const el = document.querySelector(`[data-rationale-for="${entityId}"]`);
        el?.focus();
      });
    },

    async approveEntity(entityId) {
      const form = this.getForm(entityId);
      if (form.approving) return;  // already approved, ignore duplicate calls
      try {
        await fetch(`/api/entities/${entityId}`, {
          method: 'PATCH',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ apply: { approved: true } }),
        });
        // Show in-card spinner until WS broadcasts the stage transition
        const form = this.getForm(entityId);
        form.approving = true;
        triggerConfetti();
        this._focusNext(entityId);
      } catch (e) {
        console.error('Approve failed:', e);
      }
    },

    async rejectEntity(entityId) {
      try {
        await fetch(`/api/entities/${entityId}`, {
          method: 'PATCH',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ operator_input: { instruction: 'reset' } }),
        });
      } catch (e) {
        console.error('Reject failed:', e);
      }
    },

    async unskipEntity(entityId) {
      try {
        await fetch(`/api/entities/${entityId}`, {
          method: 'PATCH',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ skip: { active: false }, operator_input: { instruction: 'reset' } }),
        });
      } catch (e) {
        console.error('Unskip failed:', e);
      }
    },

    // -------------------------------------------------------------------------
    // Helpers for templates
    // -------------------------------------------------------------------------
    getStage(entity) { return getEntityStage(entity); },

    stageColor(stageId) {
      const stage = STAGE_MAP[stageId];
      return stage ? interpolateRainbow(stage.t) : '#6b7280';
    },

    formatDate(iso) { return formatDate(iso); },

    stripAnsi(str) { return stripAnsi(str); },

    recChipClass(ops) {
      const op = (ops || '').toLowerCase();
      if (op.includes('delete') || op.includes('trash'))
        return 'bg-red-900/40 border border-red-700/40 text-red-300';
      if (op.includes('archive'))
        return 'bg-orange-900/40 border border-orange-700/40 text-orange-300';
      if (op.includes('move') || op.includes('label'))
        return 'bg-indigo-900/40 border border-indigo-700/40 text-indigo-300';
      if (op.includes('skip'))
        return 'bg-gray-700/60 border border-gray-600/40 text-gray-400';
      if (op.includes('proceed') || op.includes('keep'))
        return 'bg-green-900/40 border border-green-700/40 text-green-300';
      return 'bg-gray-700/60 border border-gray-600/40 text-gray-300';
    },

    confMeterColor(c) {
      const n = parseFloat(c);
      if (isNaN(n)) return '#6b7280';
      if (n >= 75) return '#22c55e';  // green
      if (n >= 40) return '#eab308';  // yellow
      return '#ef4444';               // red
    },

    entitySummaryFields(entity) {
      const out = {};
      if (entity.envelope) {
        out['from'] = entity.envelope.from;
        out['subject'] = entity.envelope.subject;
        out['date'] = entity.envelope.date;
      }
      if (entity.fingerprint) {
        out['keywords'] = entity.fingerprint.keywords;
        out['reader_value'] = entity.fingerprint.reader_value;
      }
      if (entity.recommendation) {
        out['rec.operations'] = entity.recommendation.operations;
        out['rec.confidence'] = entity.recommendation.confidence;
        out['rec.rationale'] = entity.recommendation.rationale;
      }
      if (entity.operator_input) {
        out['op.instruction'] = entity.operator_input.instruction;
        out['op.rationale'] = entity.operator_input.rationale;
      }
      if (entity.operator) out['cmd'] = entity.operator.command;
      if (entity.journal) {
        out['action_taken'] = entity.journal.action_taken;
        out['rule'] = entity.journal.rule;
      }
      if (entity.plan) out['planned_at'] = entity.plan.planned_at;
      if (entity.apply) {
        out['apply.approved'] = entity.apply.approved;
        out['apply.applied_at'] = entity.apply.applied_at;
      }
      return out;
    },

    // -------------------------------------------------------------------------
    // Keyboard navigation
    // -------------------------------------------------------------------------
    handleHotkey(e) {
      if (['INPUT', 'TEXTAREA', 'SELECT'].includes(e.target.tagName)) return;

      switch (e.key) {
        case 'j': {
          // Move to next filled slot, wrapping around
          const filled = this.slots.map((s, i) => i).filter(i => !!this.slots[i].entityId);
          if (!filled.length) break;
          const curSlot = this.slots.findIndex(s => s.entityId === this.focusedId);
          const curPos = filled.indexOf(curSlot);
          const nextPos = (curPos + 1) % filled.length;
          this.focusedId = this.slots[filled[nextPos]].entityId;
          break;
        }
        case 'k': {
          // Move to prev filled slot, wrapping around
          const filled = this.slots.map((s, i) => i).filter(i => !!this.slots[i].entityId);
          if (!filled.length) break;
          const curSlot = this.slots.findIndex(s => s.entityId === this.focusedId);
          const curPos = filled.indexOf(curSlot);
          const prevPos = (curPos - 1 + filled.length) % filled.length;
          this.focusedId = this.slots[filled[prevPos]].entityId;
          break;
        }
        case 'p': {
          const ent = this.getEntityById(this.focusedId);
          if (ent && getEntityStage(ent).id === 'awaiting_input' && !this.getForm(ent.id).submitted)
            this.quickAction(ent.id, 'proceed');
          break;
        }
        case 's': {
          const ent = this.getEntityById(this.focusedId);
          if (ent && getEntityStage(ent).id === 'awaiting_input' && !this.getForm(ent.id).submitted)
            this.quickAction(ent.id, 'skip');
          break;
        }
        case 'd': {
          const ent = this.getEntityById(this.focusedId);
          if (ent && getEntityStage(ent).id === 'awaiting_input' && !this.getForm(ent.id).submitted) {
            e.preventDefault();
            this.quickAction(ent.id, 'delete');
          }
          break;
        }
        case 'a': {
          const ent = this.getEntityById(this.focusedId);
          if (!ent) break;
          const stage = getEntityStage(ent).id;
          if (stage === 'awaiting_input' && !this.getForm(ent.id).submitted)
            this.quickAction(ent.id, 'archive');
          else if (stage === 'awaiting_approval' && !this.getForm(ent.id).approving)
            this.approveEntity(ent.id);
          break;
        }
        case 'r': {
          const ent = this.getEntityById(this.focusedId);
          if (ent && getEntityStage(ent).id === 'skipped')
            this.unskipEntity(ent.id);
          break;
        }
        case 'f':
          e.preventDefault();
          this.filter = this.filter === 'awaiting_input' ? null : 'awaiting_input';
          this.focusedId = null;
          break;
        case 'Escape':
          this.filter = null;
          this.focusedId = null;
          break;
      }
    },

    getEntityById(id) {
      return id ? this.entities.find(e => e.id === id) ?? null : null;
    },

    _syncSlots() {
      const filteredIds = new Set(this.filteredEntities.map(e => e.id));

      // 1. Vacate slots whose entity left the filtered list
      let vacated = false;
      for (const slot of this.slots) {
        if (slot.entityId && !filteredIds.has(slot.entityId)) {
          slot.entityId = null;
          vacated = true;
        }
      }

      // 2. Backfill empty slots — delayed if we just vacated so fade-out plays first
      const fill = () => {
        const assignedIds = new Set(this.slots.filter(s => s.entityId).map(s => s.entityId));
        const unassigned = this.filteredEntities.filter(e => !assignedIds.has(e.id));
        for (const slot of this.slots) {
          if (!slot.entityId && unassigned.length > 0) {
            slot.entityId = unassigned.shift().id;
          }
        }
        // Auto-focus first filled slot if nothing is focused
        if (!this.focusedId || !filteredIds.has(this.focusedId)) {
          const first = this.slots.find(s => s.entityId);
          if (first) this.focusedId = first.entityId;
        }
      };

      if (vacated) {
        setTimeout(fill, 250); // let fade-out play before new card fades in
      } else {
        fill();
      }
    },

    _focusNext(entityId) {
      // Move focus to the next filled slot after the given entity's slot
      const curSlotIdx = this.slots.findIndex(s => s.entityId === entityId);
      const filled = this.slots.map((s, i) => i).filter(i => !!this.slots[i].entityId && this.slots[i].entityId !== entityId);
      if (!filled.length) { this.focusedId = null; return; }
      const next = filled.find(i => i > curSlotIdx) ?? filled[0];
      this.focusedId = this.slots[next].entityId;
    },
  };
}
