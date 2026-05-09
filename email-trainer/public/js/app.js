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
    tombstones: {},        // entity IDs that left the current filter (holds card slot as blank placeholder)

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
      return sorted.filter(e => getEntityStage(e).id === this.filter || this.tombstones[e.id]);
    },

    get focusedIdx() {
      if (!this.focusedId) return 0;
      const idx = this.filteredEntities.findIndex(e => e.id === this.focusedId);
      return idx >= 0 ? idx : 0;
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
      this.$watch('filter', () => { this.tombstones = {}; });
    },

    async loadConfig() {
      try {
        const res = await fetch('/api/config');
        const cfg = await res.json();
        if (cfg.destinations?.length) this.cfgDestinations = cfg.destinations;
      } catch { }
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
            // If entity was in the active filter but is no longer, mark as tombstone
            const wasInFilter = this.filter && getEntityStage(this.entities[idx]).id === this.filter;
            const isInFilter = this.filter && getEntityStage(msg.entity).id === this.filter;
            if (wasInFilter && !isInFilter) this.tombstones[msg.entity.id] = true;
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
            if (this.tombstones[msg.id]) {
              // Already a placeholder — remove immediately, no animation needed
              delete this.tombstones[msg.id];
              this.entities.splice(idx, 1);
              delete this.forms[msg.id];
            } else {
              this.entities[idx]._deleting = true;
              setTimeout(() => {
                const i = this.entities.findIndex(e => e.id === msg.id);
                if (i >= 0) this.entities.splice(i, 1);
                delete this.forms[msg.id];
              }, 420);
            }
          }
          break;
        }
      }
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
        // The tombstone system handles the card disappearing; resetting here causes a
        // brief form re-flash before the stage change is broadcast.
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
      const entities = this.filteredEntities;
      if (!entities.length) return;

      switch (e.key) {
        case 'j': {
          const next = Math.min(this.focusedIdx + 1, entities.length - 1);
          this.focusedId = entities[next].id;
          break;
        }
        case 'k': {
          const prev = Math.max(this.focusedIdx - 1, 0);
          this.focusedId = entities[prev].id;
          break;
        }
        case 'p': {
          const ent = entities[this.focusedIdx];
          if (ent && getEntityStage(ent).id === 'awaiting_input' && !this.getForm(ent.id).submitted)
            this.quickAction(ent.id, 'proceed');
          break;
        }
        case 's': {
          const ent = entities[this.focusedIdx];
          if (ent && getEntityStage(ent).id === 'awaiting_input' && !this.getForm(ent.id).submitted)
            this.quickAction(ent.id, 'skip');
          break;
        }
        case 'd': {
          const ent = entities[this.focusedIdx];
          if (ent && getEntityStage(ent).id === 'awaiting_input' && !this.getForm(ent.id).submitted) {
            e.preventDefault();
            this.quickAction(ent.id, 'delete');
          }
          break;
        }
        case 'a': {
          const ent = entities[this.focusedIdx];
          if (ent && getEntityStage(ent).id === 'awaiting_approval' && !this.getForm(ent.id).approving)
            this.approveEntity(ent.id);
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

    _focusNext(entityId) {
      const entities = this.filteredEntities.filter(e => !this.tombstones[e.id]);
      const idx = entities.findIndex(e => e.id === entityId);
      const next = idx !== -1 && idx < entities.length - 1 ? idx + 1 : idx;
      if (next >= 0) this.focusedId = entities[next].id;
    },
  };
}
