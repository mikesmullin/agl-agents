// email-trainer-2/public/js/app.js
// Alpine.js application — Gmail-style email triage UI

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
  if ((e.operator_input?.processed) || e.execution || e.journal) return STAGE_MAP['processing'];
  if (e.operator_input && e.operator_input.instruction !== null && e.operator_input.instruction !== undefined) {
    return STAGE_MAP['processing'];
  }
  if (e.operator_input && e.operator_input.instruction === null) return STAGE_MAP['awaiting_input'];
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
      month: 'short', day: 'numeric', year: 'numeric', hour: '2-digit', minute: '2-digit',
    });
  } catch { return iso; }
}

// =============================================================================
// Main Alpine.js app
// =============================================================================
function app() {
  return {
    entities: [],
    filter: null,            // stage id or null (show all)
    ws: null,
    wsStatus: 'connecting',
    showAllStages: false,
    cfgDestinations: [],
    focusedId: null,         // row under keyboard/hover focus
    searchQuery: '',

    // Selection (Gmail-style multi-select)
    selected: {},            // id -> true
    anchorId: null,          // last clicked row, anchor for shift-range select
    starred: {},             // id -> true (localStorage)
    important: {},           // id -> true (localStorage)

    // Detail view
    openId: null,            // open email id, or null for list
    rawHtml: null,
    rawText: null,
    rawMode: 'html',
    rawLoading: false,
    _applyingHash: false,    // guard to avoid hash/state feedback loops

    // Toolbar action / secondary toolbar
    pendingAction: null,     // 'proceed'|'spam'|'delete'|'archive'|'move'|'notice'|null
    secondaryOpen: false,
    submitting: false,
    actionError: null,
    actionFlash: null,       // transient success confirmation
    refreshing: false,
    form: { instruction: '', rationale: '', destination: '', notice_capture: '', notice_display: '' },

    // ----- Training section state (preserved from v1) -----
    view: 'triage',          // 'triage' | 'operator-history' | 'trial-history' | 'trial-run'
    archiveEntries: [],
    archiveLoading: false,
    archiveSortCol: 'apply.applied_at',
    archiveSortDir: -1,
    archiveCopied: null,
    archiveSearch: '',
    archiveFilterInstruction: '',
    archiveDeleteId: null,

    trialEntries: [],
    trialLoading: false,
    trialSortCol: 'id',
    trialSortDir: -1,
    trialCopied: null,
    trialDeleteId: null,
    trialPromoteId: null,
    trialPromoting: false,
    trialPromoteResult: null,
    trialChartMetric: 'passing',

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
      const sorted = [...this.entities].sort((a, b) => {
        const da = a.envelope?.date ? new Date(a.envelope.date).getTime() : 0;
        const db = b.envelope?.date ? new Date(b.envelope.date).getTime() : 0;
        return db - da; // newest first (Gmail-style)
      });
      let list = this.filter ? sorted.filter(e => getEntityStage(e).id === this.filter) : sorted;
      const q = this.searchQuery.trim().toLowerCase();
      if (q) {
        list = list.filter(e => [
          e.envelope?.from, e.envelope?.subject,
          e.summary?.headline, e.summary?.description, e.summary?.text,
          e.recommendation?.operations, e.id,
        ].some(v => v != null && String(v).toLowerCase().includes(q)));
      }
      return list;
    },

    get focusedStageId() {
      const ent = this.getEntityById(this.focusedId);
      return ent ? getEntityStage(ent).id : null;
    },

    get openEntity() { return this.getEntityById(this.openId); },

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

    get selectedCount() { return Object.keys(this.selected).length; },

    // Toolbar contextual visibility for the Proceed / Approve buttons
    get showProceedBtn() { return !this.filter || ['awaiting_input', 'skipped'].includes(this.filter); },
    get showApproveBtn() { return !this.filter || this.filter === 'awaiting_approval'; },

    get allSelected() {
      const f = this.filteredEntities;
      return f.length > 0 && f.every(e => this.selected[e.id]);
    },

    // -------------------------------------------------------------------------
    // Init
    // -------------------------------------------------------------------------
    init() {
      this.loadFlags();
      this.loadEntities();   // fast initial REST load — don't wait on the WebSocket
      this.connectWS();      // live updates (and a redundant init payload) thereafter
      this.loadConfig();
      document.addEventListener('keydown', e => this.handleHotkey(e));

      // Hash routing + browser history. Apply the current hash, then keep the
      // hash in sync with view/filter/openId so Back/Forward work naturally.
      this._onHashChange();
      window.addEventListener('hashchange', () => this._onHashChange());
      this.$watch('view', () => this._syncHash());
      this.$watch('filter', () => this._syncHash());
      this.$watch('openId', () => this._syncHash());
    },

    // -------------------------------------------------------------------------
    // Hash routing
    //   #/                     → inbox (all)
    //   #/stage/<stageId>      → inbox filtered to a stage
    //   #/email/<id>           → email detail
    //   #/operator-history     #/trial-history     #/trial-run
    // -------------------------------------------------------------------------
    _buildHash() {
      if (this.view === 'operator-history') return '#/operator-history';
      if (this.view === 'trial-history') return '#/trial-history';
      if (this.view === 'trial-run') return '#/trial-run';
      if (this.openId) return '#/email/' + encodeURIComponent(this.openId);
      if (this.filter) return '#/stage/' + encodeURIComponent(this.filter);
      return '#/';
    },

    _syncHash() {
      if (this._applyingHash) return;
      const target = this._buildHash();
      if (location.hash !== target) location.hash = target;
    },

    _onHashChange() {
      const parts = location.hash.replace(/^#\/?/, '').split('/').filter(s => s !== '');
      const head = parts[0] || '';
      this._applyingHash = true;
      try {
        if (head === 'operator-history' || head === 'trial-history' || head === 'trial-run') {
          this.openId = null;
          this.setView(head);
        } else if (head === 'email' && parts[1]) {
          const id = decodeURIComponent(parts[1]);
          this.view = 'triage';
          if (this.openId !== id) this.openEmail(id);
        } else if (head === 'stage' && parts[1]) {
          this.view = 'triage';
          this.openId = null;
          this.filter = decodeURIComponent(parts[1]);
        } else {
          this.view = 'triage';
          this.openId = null;
          this.filter = null;
        }
      } finally {
        this.$nextTick(() => { this._applyingHash = false; });
      }
    },

    // Initial population via REST so the list appears immediately, independent
    // of how long the WebSocket handshake/init push takes.
    async loadEntities() {
      try {
        const res = await fetch('/api/entities');
        if (res.ok) this.entities = await res.json();
      } catch { }
    },

    async loadConfig() {
      try {
        const res = await fetch('/api/config');
        const cfg = await res.json();
        if (cfg.destinations?.length) this.cfgDestinations = cfg.destinations;
      } catch { }
    },

    // -------------------------------------------------------------------------
    // Star / important flags (client-side only, localStorage)
    // -------------------------------------------------------------------------
    loadFlags() {
      try { this.starred = JSON.parse(localStorage.getItem('et2_starred') || '{}'); } catch { this.starred = {}; }
      try { this.important = JSON.parse(localStorage.getItem('et2_important') || '{}'); } catch { this.important = {}; }
    },
    isStarred(id) { return !!this.starred[id]; },
    isImportant(id) { return !!this.important[id]; },
    toggleStar(id) {
      const next = { ...this.starred };
      if (next[id]) delete next[id]; else next[id] = true;
      this.starred = next;
      localStorage.setItem('et2_starred', JSON.stringify(next));
    },
    toggleImportant(id) {
      const next = { ...this.important };
      if (next[id]) delete next[id]; else next[id] = true;
      this.important = next;
      localStorage.setItem('et2_important', JSON.stringify(next));
    },

    // -------------------------------------------------------------------------
    // Selection
    // -------------------------------------------------------------------------
    // Modifier-aware: Shift = range select from anchor, plain = toggle one.
    toggleSelect(id, event) {
      this.focusedId = id;  // selecting a row also makes it the active row
      if (event?.shiftKey && this.anchorId && this.anchorId !== id) {
        this._selectRange(this.anchorId, id);
        return;
      }
      const next = { ...this.selected };
      if (next[id]) delete next[id]; else next[id] = true;
      this.selected = next;
      this.anchorId = id;
    },
    _selectRange(fromId, toId) {
      const list = this.filteredEntities;
      const i = list.findIndex(e => e.id === fromId);
      const j = list.findIndex(e => e.id === toId);
      if (i < 0 || j < 0) { this.toggleSelect(toId); return; }
      const [a, b] = i < j ? [i, j] : [j, i];
      const next = { ...this.selected };
      for (let k = a; k <= b; k++) next[list[k].id] = true;
      this.selected = next;
      this.anchorId = toId;
      this.focusedId = toId;
    },
    // Click on a row's sender / summary / preview text.
    // Shift or Ctrl/Cmd → select instead of opening the detail view.
    rowClick(entity, event) {
      this.focusedId = entity.id;
      if (event?.shiftKey) { this.toggleSelect(entity.id, event); return; }
      if (event?.ctrlKey || event?.metaKey) { this.toggleSelect(entity.id); return; }
      this.openEmail(entity.id);
    },
    clearSelection() { this.selected = {}; this.anchorId = null; },
    toggleSelectAll() {
      if (this.allSelected) { this.clearSelection(); return; }
      const next = {};
      this.filteredEntities.forEach(e => next[e.id] = true);
      this.selected = next;
    },

    // -------------------------------------------------------------------------
    // Refresh — reload latest entity state on demand
    // -------------------------------------------------------------------------
    async refresh() {
      this.refreshing = true;
      try {
        const res = await fetch('/api/entities');
        if (res.ok) this.entities = await res.json();
      } catch { }
      setTimeout(() => { this.refreshing = false; }, 450);
    },

    // -------------------------------------------------------------------------
    // Detail view
    // -------------------------------------------------------------------------
    async openEmail(id) {
      this.openId = id;
      this.focusedId = id;
      this.rawHtml = null;
      this.rawText = null;
      this.rawLoading = true;
      this.rawMode = 'html';
      try {
        const res = await fetch(`/api/entities/${encodeURIComponent(id)}/raw`);
        const d = await res.json();
        if (d.contentType === 'html' && d.content) {
          this.rawHtml = d.content;
          this.rawMode = 'html';
        } else {
          this.rawHtml = null;
          this.rawMode = 'text';
        }
        this.rawText = d.text || d.content || '';
      } catch {
        this.rawText = 'Failed to load original email.';
        this.rawMode = 'text';
      }
      this.rawLoading = false;
    },

    // -------------------------------------------------------------------------
    // Toolbar actions
    // -------------------------------------------------------------------------
    actionMeta(action) {
      switch (action) {
        case 'proceed': return { label: '▶ Proceed', color: 'text-green-400' };
        case 'skip': return { label: '⏭️ Skip', color: 'text-gray-300' };
        case 'spam': return { label: '🚫 Spam', color: 'text-red-400' };
        case 'delete': return { label: '🗑️ Delete', color: 'text-red-400' };
        case 'archive': return { label: '📦 Archive', color: 'text-orange-400' };
        case 'move': return { label: '📁 Move', color: 'text-indigo-400' };
        case 'notice': return { label: '📢 Notice', color: 'text-indigo-300' };
        default: return { label: '', color: 'text-gray-400' };
      }
    },

    openAction(action) {
      this.pendingAction = action;
      this.secondaryOpen = true;
      this.actionError = null;
      // Prefill instruction with the action keyword (editable)
      const presets = { proceed: 'proceed', skip: 'skip', spam: 'spam', delete: 'delete', archive: 'archive', move: '', notice: '' };
      this.form.instruction = presets[action] ?? '';
      if (action !== 'move') this.form.destination = '';
      // Focus the instruction box for quick editing / Enter-to-submit
      this.$nextTick(() => {
        const el = document.querySelector('[data-instruction-box]');
        el?.focus();
        el?.select?.();
      });
    },

    // Proceed is special: it never needs a rationale, so it submits immediately
    // without opening the secondary toolbar.
    async proceedNow() {
      if (this.submitting) return;
      this.secondaryOpen = false;
      this.pendingAction = null;
      const targets = this.actionTargets();
      if (!targets.length) {
        this.actionFlash = null;
        this.actionError = '⚠️ Select at least one email first';
        setTimeout(() => { this.actionError = null; }, 3000);
        return;
      }
      this.submitting = true;
      this.actionError = null;
      this.actionFlash = null;
      let applied = 0;
      for (const e of targets) {
        if (await this.hotkeyAction(e, 'proceed')) applied++;
      }
      this.submitting = false;
      this.clearSelection();
      if (this.openId) this.openId = null;  // acted from detail page → back to list
      if (applied > 0) {
        this.actionFlash = `✅ Proceeded ${applied} email${applied > 1 ? 's' : ''}`;
        setTimeout(() => { this.actionFlash = null; }, 2500);
      } else {
        this.actionError = '⚠️ No emails ready to proceed at their current stage';
        setTimeout(() => { this.actionError = null; }, 3000);
      }
    },

    // Approve is also rationale-free and submits immediately. It only affects
    // selected emails that are at the awaiting-approval gate.
    async approveNow() {
      if (this.submitting) return;
      this.secondaryOpen = false;
      this.pendingAction = null;
      const targets = this.actionTargets();
      if (!targets.length) {
        this.actionFlash = null;
        this.actionError = '⚠️ Select at least one email first';
        setTimeout(() => { this.actionError = null; }, 3000);
        return;
      }
      this.submitting = true;
      this.actionError = null;
      this.actionFlash = null;
      let applied = 0;
      for (const e of targets) {
        if (getEntityStage(e).id === 'awaiting_approval') {
          if (await this.patch(e.id, { apply: { approved: true } })) applied++;
        }
      }
      this.submitting = false;
      this.clearSelection();
      if (this.openId) this.openId = null;  // acted from detail page → back to list
      if (applied > 0) {
        this.actionFlash = `✅ Approved ${applied} email${applied > 1 ? 's' : ''}`;
        setTimeout(() => { this.actionFlash = null; }, 2500);
      } else {
        this.actionError = '⚠️ No emails awaiting approval in selection';
        setTimeout(() => { this.actionError = null; }, 3000);
      }
    },

    toggleNotice() {
      if (this.pendingAction === 'notice') {
        this.pendingAction = this._prevAction || 'delete';
        const presets = { spam: 'spam', delete: 'delete', archive: 'archive', move: '' };
        this.form.instruction = presets[this.pendingAction] ?? '';
      } else {
        this._prevAction = this.pendingAction;
        this.pendingAction = 'notice';
        this.form.instruction = '';
      }
      this.secondaryOpen = true;
    },

    closeAction() {
      this.secondaryOpen = false;
      this.pendingAction = null;
      this.actionError = null;
      this.actionFlash = null;
      this.form = { instruction: '', rationale: '', destination: '', notice_capture: '', notice_display: '' };
    },

    // Which entities a toolbar action will affect: the current selection, or
    // the open email when in the detail view. Toolbar actions are selection-
    // driven (the keyboard hotkeys cover single focused-row actions).
    actionTargets() {
      if (this.selectedCount > 0) {
        return this.entities.filter(e => this.selected[e.id]);
      }
      if (this.openId) {
        const e = this.getEntityById(this.openId);
        return e ? [e] : [];
      }
      return [];
    },

    actionTargetSummary() {
      const n = this.actionTargets().length;
      if (n === 0) return 'no target — select emails first';
      if (this.selectedCount > 0) return `${n} selected email${n > 1 ? 's' : ''}`;
      return '1 open email';
    },

    async submitAction() {
      if (this.submitting) return;
      const action = this.pendingAction;
      const targets = this.actionTargets();
      if (!targets.length) {
        this.actionError = '⚠️ Select at least one email first';
        setTimeout(() => { this.actionError = null; }, 3000);
        return;
      }
      if (action === 'move' && !this.form.destination && !this.form.instruction.trim()) {
        this.actionError = '⚠️ Choose a destination folder';
        setTimeout(() => { this.actionError = null; }, 3000);
        return;
      }
      this.submitting = true;
      this.actionError = null;
      this.actionFlash = null;
      let applied = 0;
      for (const e of targets) {
        const ok = await this.applyToEntity(e, action);
        if (ok) applied++;
      }
      this.submitting = false;
      this.clearSelection();
      if (this.openId) this.openId = null;  // acted from detail page → back to list
      // Reset the form values and collapse the secondary toolbar after submit.
      this.closeAction();
      if (applied > 0) {
        this.actionFlash = `✅ Submitted to ${applied} email${applied > 1 ? 's' : ''}`;
        setTimeout(() => { this.actionFlash = null; }, 2500);
      } else {
        this.actionError = '⚠️ No matching emails for this action at their current stage';
        setTimeout(() => { this.actionError = null; }, 3000);
      }
    },

    // Apply a toolbar action to a single entity according to its stage.
    // Returns true if a patch was sent.
    async applyToEntity(entity, action) {
      const stage = getEntityStage(entity).id;
      const rationale = this.form.rationale.trim() || null;

      if (action === 'proceed') {
        if (stage === 'awaiting_approval') return this.patch(entity.id, { apply: { approved: true } });
        if (stage === 'skipped') return this.patch(entity.id, { skip: { active: false }, operator_input: { instruction: 'reset' } });
        if (stage === 'awaiting_input') return this.patch(entity.id, { operator_input: { instruction: this.form.instruction.trim() || 'proceed', rationale } });
        return false;
      }

      // All other actions are operator-input instructions — only valid at the input gate
      if (stage !== 'awaiting_input') return false;

      let instruction;
      if (action === 'move') instruction = this.form.instruction.trim() || `move to ${this.form.destination}`;
      else if (action === 'notice') instruction = this.form.instruction.trim() || 'notice';
      else instruction = this.form.instruction.trim() || action;

      const patch = { operator_input: { instruction, rationale } };
      if (action === 'notice') {
        patch.operator_input.notice_capture = this.form.notice_capture.trim() || null;
        patch.operator_input.notice_display = this.form.notice_display.trim() || null;
      }
      return this.patch(entity.id, patch);
    },

    async patch(id, body) {
      try {
        const res = await fetch(`/api/entities/${encodeURIComponent(id)}`, {
          method: 'PATCH',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(body),
        });
        return res.ok;
      } catch (e) {
        console.error('PATCH failed:', e);
        return false;
      }
    },

    // Immediate single-entity action for hotkeys (no secondary toolbar)
    async hotkeyAction(entity, action) {
      const stage = getEntityStage(entity).id;
      if (action === 'proceed') {
        if (stage === 'awaiting_approval') return this.patch(entity.id, { apply: { approved: true } });
        if (stage === 'skipped') return this.patch(entity.id, { skip: { active: false }, operator_input: { instruction: 'reset' } });
        if (stage === 'awaiting_input') return this.patch(entity.id, { operator_input: { instruction: 'proceed' } });
        return;
      }
      if (action === 'reprocess') {
        return this.patch(entity.id, { skip: { active: false }, operator_input: { instruction: 'reset' } });
      }
      if (stage !== 'awaiting_input') return;
      return this.patch(entity.id, { operator_input: { instruction: action } });
    },

    // -------------------------------------------------------------------------
    // Sidebar
    // -------------------------------------------------------------------------
    clickStageFilter(stageId) {
      this.filter = this.filter === stageId ? null : stageId;
      this.openId = null;
      this.focusedId = null;
    },
    clickEyeToggle() { this.showAllStages = !this.showAllStages; },

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
        this.ws.onclose = () => { this.wsStatus = 'disconnected'; setTimeout(() => this.connectWS(), 3000); };
        this.ws.onerror = () => { this.wsStatus = 'disconnected'; };
        this.ws.onmessage = (evt) => { try { this.handleWSMessage(JSON.parse(evt.data)); } catch { } };
      } catch { }
    },

    handleWSMessage(msg) {
      switch (msg.type) {
        case 'init':
          this.entities = msg.entities || [];
          break;
        case 'entity:new': {
          if (!this.entities.find(e => e.id === msg.entity.id)) this.entities.push(msg.entity);
          break;
        }
        case 'entity:modified': {
          const idx = this.entities.findIndex(e => e.id === msg.entity.id);
          if (idx >= 0) this.entities[idx] = msg.entity;
          else this.entities.push(msg.entity);
          break;
        }
        case 'entity:deleted': {
          const idx = this.entities.findIndex(e => e.id === msg.id);
          if (idx >= 0) {
            this.entities[idx]._deleting = true;
            setTimeout(() => {
              const i = this.entities.findIndex(e => e.id === msg.id);
              if (i >= 0) this.entities.splice(i, 1);
              if (this.selected[msg.id]) { const n = { ...this.selected }; delete n[msg.id]; this.selected = n; }
              if (this.openId === msg.id) this.openId = null;
            }, 400);
          }
          break;
        }
      }
    },

    // -------------------------------------------------------------------------
    // Template helpers
    // -------------------------------------------------------------------------
    getEntityById(id) { return id ? this.entities.find(e => e.id === id) ?? null : null; },
    getStage(entity) { return getEntityStage(entity); },
    isGated(entity) { const s = getEntityStage(entity).id; return s === 'awaiting_input' || s === 'awaiting_approval'; },

    senderName(entity) {
      const from = entity.envelope?.from || '';
      const m = from.match(/^\s*"?([^"<]+?)"?\s*<([^>]+)>\s*$/);
      if (m) return m[1].trim();
      return from || entity.envelope?.senderEmail || '—';
    },

    stageColor(stageId) {
      const stage = STAGE_MAP[stageId];
      return stage ? interpolateRainbow(stage.t) : '#6b7280';
    },

    formatDate(iso) { return formatDate(iso); },
    stripAnsi(str) { return stripAnsi(str); },

    relativeTime(iso) {
      if (!iso) return '—';
      const diff = Date.now() - new Date(iso).getTime();
      const s = Math.round(diff / 1000);
      if (s < 60) return `${s}s`;
      const m = Math.round(s / 60);
      if (m < 60) return `${m}m`;
      const h = Math.round(m / 60);
      if (h < 24) return `${h}h`;
      const d = Math.round(h / 24);
      if (d < 30) return `${d}d`;
      const mo = Math.round(d / 30);
      if (mo < 12) return `${mo}mo`;
      return `${Math.round(mo / 12)}y`;
    },

    recChipClass(ops) {
      const op = (ops || '').toLowerCase();
      if (op.includes('delete') || op.includes('trash')) return 'bg-red-900/40 border border-red-700/40 text-red-300';
      if (op.includes('archive')) return 'bg-orange-900/40 border border-orange-700/40 text-orange-300';
      if (op.includes('move') || op.includes('label')) return 'bg-indigo-900/40 border border-indigo-700/40 text-indigo-300';
      if (op.includes('skip')) return 'bg-gray-700/60 border border-gray-600/40 text-gray-400';
      if (op.includes('proceed') || op.includes('keep')) return 'bg-green-900/40 border border-green-700/40 text-green-300';
      return 'bg-gray-700/60 border border-gray-600/40 text-gray-300';
    },

    confMeterColor(c) {
      const n = parseFloat(c);
      if (isNaN(n)) return '#6b7280';
      if (n >= 75) return '#22c55e';
      if (n >= 40) return '#eab308';
      return '#ef4444';
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
    // Keyboard
    // -------------------------------------------------------------------------
    _moveFocus(dir) {
      const list = this.filteredEntities;
      if (!list.length) return;
      let idx = list.findIndex(e => e.id === this.focusedId);
      idx = idx < 0 ? (dir > 0 ? 0 : list.length - 1) : (idx + dir + list.length) % list.length;
      this.focusedId = list[idx].id;
      this.$nextTick(() => {
        const el = document.querySelector(`.email-row.row-focused`);
        el?.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
      });
    },

    _advanceFocus() {
      const list = this.filteredEntities;
      const idx = list.findIndex(e => e.id === this.focusedId);
      if (idx >= 0 && idx + 1 < list.length) this.focusedId = list[idx + 1].id;
    },

    // Stage context for the p/a hotkeys: prefer the focused row, then the
    // active stage filter, then the first selected email (so mouse-only
    // shift-selection still resolves a stage without using j/k).
    _contextStageId() {
      const ent = this.getEntityById(this.focusedId);
      if (ent) return getEntityStage(ent).id;
      if (this.filter) return this.filter;
      const firstSel = Object.keys(this.selected)[0];
      if (firstSel) {
        const e = this.getEntityById(firstSel);
        if (e) return getEntityStage(e).id;
      }
      return null;
    },

    handleHotkey(e) {
      // Search shortcut works even outside inputs
      if (e.key === '/' && !['INPUT', 'TEXTAREA', 'SELECT'].includes(e.target.tagName)) {
        e.preventDefault();
        document.querySelector('[data-search-box]')?.focus();
        return;
      }
      if (['INPUT', 'TEXTAREA', 'SELECT'].includes(e.target.tagName)) {
        if (e.key === 'Escape') e.target.blur();
        return;
      }
      if (this.view !== 'triage') return;

      switch (e.key) {
        case 'j': e.preventDefault(); this._moveFocus(1); break;
        case 'k': e.preventDefault(); this._moveFocus(-1); break;
        case ' ': {
          e.preventDefault();
          if (this.focusedId) this.toggleSelect(this.focusedId, e); // Shift+Space = range
          break;
        }
        case 'x': {
          if (this.focusedId) this.toggleSelect(this.focusedId);
          break;
        }
        case 'Enter': {
          if (this.focusedId) { e.preventDefault(); this.openEmail(this.focusedId); }
          break;
        }
        case 'p': {
          const stage = this._contextStageId();
          if (stage === 'awaiting_input') { this.proceedNow(); break; }   // same as Proceed button
          if (stage === 'awaiting_approval') { this.approveNow(); break; } // contextual button is Approve here
          const ent = this.getEntityById(this.focusedId);
          if (ent) { this._advanceFocus(); this.hotkeyAction(ent, 'proceed'); }
          break;
        }
        case 's': {
          const ent = this.getEntityById(this.focusedId);
          if (ent && getEntityStage(ent).id === 'awaiting_input') { this._advanceFocus(); this.hotkeyAction(ent, 'skip'); }
          break;
        }
        case 'd': {
          const ent = this.getEntityById(this.focusedId);
          if (ent && getEntityStage(ent).id === 'awaiting_input') { e.preventDefault(); this._advanceFocus(); this.hotkeyAction(ent, 'delete'); }
          break;
        }
        case 'a': {
          const stage = this._contextStageId();
          if (stage === 'awaiting_approval') { this.approveNow(); break; } // same as Approve button: selected only
          const ent = this.getEntityById(this.focusedId);
          if (ent && getEntityStage(ent).id === 'awaiting_input') { this._advanceFocus(); this.hotkeyAction(ent, 'archive'); }
          break;
        }
        case 'r': {
          const ent = this.getEntityById(this.focusedId);
          if (ent && getEntityStage(ent).id === 'skipped') this.hotkeyAction(ent, 'reprocess');
          break;
        }
        case 'Escape':
          if (this.secondaryOpen) this.closeAction();
          else if (this.openId) this.openId = null;
          else if (this.searchQuery) this.searchQuery = '';
          else { this.filter = null; this.focusedId = null; this.clearSelection(); }
          break;
      }
    },

    // =========================================================================
    // Training views (preserved from v1)
    // =========================================================================
    setView(v) {
      this.view = v;
      if (v === 'triage') { /* keep state */ }
      if (v === 'operator-history' && this.archiveEntries.length === 0) this.loadArchive();
      if (v === 'trial-history' && this.trialEntries.length === 0) this.loadTrials();
      if (v === 'trial-run') {
        this.loadTrialRunStatus();
        if (!this.trialRunPollInterval) {
          this.trialRunPollInterval = setInterval(() => {
            if (this.view === 'trial-run') this.loadTrialRunStatus();
            else { clearInterval(this.trialRunPollInterval); this.trialRunPollInterval = null; }
          }, 5000);
        }
      } else if (this.trialRunPollInterval && v !== 'trial-run') {
        clearInterval(this.trialRunPollInterval);
        this.trialRunPollInterval = null;
      }
    },

    // ----- Operator History (archive) -----
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
      if (this.archiveFilterInstruction) {
        entries = entries.filter(e => (e.operator_input?._parsed_operation ?? '') === this.archiveFilterInstruction);
      }
      return entries;
    },

    get sortedArchiveEntries() {
      const col = this.archiveSortCol, dir = this.archiveSortDir;
      return [...this.filteredArchiveEntries].sort((a, b) => {
        const av = this._archiveVal(a, col), bv = this._archiveVal(b, col);
        if (av < bv) return -dir;
        if (av > bv) return dir;
        return 0;
      });
    },

    get archiveInstructionOptions() {
      const set = new Set();
      this.archiveEntries.forEach(e => { const v = e.operator_input?._parsed_operation; if (v) set.add(v); });
      return [...set].sort();
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
      return `Last confirmed: ${new Date(ts).toLocaleString()} (${this.relativeTime(ts)} ago)`;
    },

    archiveSortBy(col) {
      if (this.archiveSortCol === col) this.archiveSortDir = -this.archiveSortDir;
      else { this.archiveSortCol = col; this.archiveSortDir = -1; }
    },
    archiveSortIcon(col) { return this.archiveSortCol !== col ? '↕' : (this.archiveSortDir === -1 ? '↓' : '↑'); },

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
        if (res.ok) this.archiveEntries = this.archiveEntries.filter(e => e.id !== id);
      } catch { }
    },

    // ----- Trial History -----
    get trialChartEntries() {
      const metric = this.trialChartMetric;
      return [...this.trialEntries]
        .filter(t => this._trialChartVal(t, metric) != null)
        .sort((a, b) => String(a.id).localeCompare(String(b.id)));
    },
    get maxTrialChartVal() {
      const vals = this.trialChartEntries.map(t => this._trialChartVal(t, this.trialChartMetric));
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
      const col = this.trialSortCol, dir = this.trialSortDir;
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
      if (this.trialSortCol === col) this.trialSortDir = -this.trialSortDir;
      else { this.trialSortCol = col; this.trialSortDir = -1; }
    },
    trialSortIcon(col) { return this.trialSortCol !== col ? '↕' : (this.trialSortDir === -1 ? '↓' : '↑'); },
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
      try { const res = await fetch('/api/trials'); this.trialEntries = res.ok ? await res.json() : []; }
      catch { this.trialEntries = []; }
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
      try { const res = await fetch(`/api/trials/${encodeURIComponent(id)}`, { method: 'DELETE' }); if (res.ok) this.trialEntries = this.trialEntries.filter(e => e.id !== id); }
      catch { }
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
      } catch (e) { this.trialPromoteResult = { ok: false, error: e.message }; }
      this.trialPromoting = false;
      if (this.trialPromoteResult?.ok) setTimeout(() => { this.trialPromoteId = null; this.trialPromoteResult = null; }, 2000);
    },

    // ----- Trial Run -----
    async loadTrialRunStatus() {
      this.trialRunLoading = true;
      try { const res = await fetch('/api/trial-run/status'); if (res.ok) this.trialRunStatus = await res.json(); }
      catch { }
      this.trialRunLoading = false;
    },
    async startTrialRun() {
      this.trialRunStarting = true;
      this.trialRunError = null;
      try {
        const body = this.trialRunFromId.trim() ? { fromId: this.trialRunFromId.trim() } : {};
        const res = await fetch('/api/trial-run/start', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) });
        const data = await res.json();
        if (!res.ok) { this.trialRunError = data.error || `Error ${res.status}`; this.trialRunStarting = false; }
        else {
          const poll = async (attempts) => {
            await this.loadTrialRunStatus();
            if (this.trialRunStatus.running) this.trialRunStarting = false;
            else if (attempts > 0) setTimeout(() => poll(attempts - 1), 1000);
            else this.trialRunStarting = false;
          };
          setTimeout(() => poll(5), 1000);
        }
      } catch (e) { this.trialRunError = e.message; this.trialRunStarting = false; }
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
  };
}
