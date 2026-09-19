import { invoke } from '@tauri-apps/api/core';
import { defineStore } from 'pinia';
import { computed, ref } from 'vue';

import {
  defaultTargetSelection,
  mcpViewIndex,
  newIdempotencyKey,
  parseCapabilityError
} from '@/libs/capability.js';

/**
 * @typedef {Object} CapabilityTarget
 * @property {string} id - Stable target id (kebab-case)
 * @property {boolean} default_selected - Selected when the user chooses nothing
 * @property {boolean} external - Whether the target lives outside ChatSpeed
 * @property {boolean} supported - Whether a verified directory exists
 * @property {string} [path] - Absolute install directory, when supported
 * @property {string} [unsupported_reason] - Why the target cannot be used
 * @property {string} [verified_against] - The convention the path was verified against
 */

/**
 * @typedef {Object} CapabilitySkillEntry
 * @property {string} name
 * @property {string} version
 * @property {string} description
 * @property {string} source - builtin | managed | discovered
 * @property {string} [target_id]
 * @property {string} [directory]
 * @property {boolean} present
 * @property {boolean} managed
 * @property {boolean} drifted
 * @property {boolean} protected
 * @property {boolean} uninstallable
 * @property {string} [installation_state]
 * @property {string} [checker_version]
 * @property {string} [verdict]
 */

/**
 * @typedef {Object} CapabilityCheckReport
 * @property {string} checker_version
 * @property {string} verdict - pass | blocked | inconclusive
 * @property {Array<{rule: string, severity: string, path?: string, detail: string}>} findings
 * @property {Record<string, boolean|string>} permissions
 * @property {string} [skill_name]
 * @property {string} [content_digest]
 * @property {number} file_count
 * @property {number} total_bytes
 * @property {string} source_kind
 * @property {string} source_ref
 */

/**
 * @typedef {Object} CapabilityTargetOutcome
 * @property {string} target_id
 * @property {string} status - installed | already_installed | skipped_existing | blocked | unsupported | failed
 * @property {string} [install_path]
 * @property {string} [installation_id]
 * @property {string} [detail]
 */

/**
 * @typedef {Object} CapabilityUninstallOutcome
 * @property {string} target_id
 * @property {string} skill_name
 * @property {string} status - removed | finalized | refused | not_found
 * @property {string} [install_path]
 * @property {string} [quarantine_path]
 * @property {string} [detail]
 */

/**
 * @typedef {Object} CapabilityMutationResult
 * @property {string} operation_id
 * @property {boolean} replayed
 * @property {Object} result - The redacted project recorded in the journal
 */

/**
 * @param {string} command
 * @param {Record<string, unknown>} [payload]
 */
async function invokeCapability(command, payload = {}) {
  try {
    return await invoke(command, payload);
  } catch (error) {
    throw parseCapabilityError(error);
  }
}

export const useCapabilityStore = defineStore('capability', () => {
  /** @type {import('vue').Ref<CapabilityTarget[]>} */
  const targets = ref([]);
  /** @type {import('vue').Ref<CapabilitySkillEntry[]>} */
  const skills = ref([]);
  /** @type {import('vue').Ref<Record<number, Object>>} */
  const mcpViews = ref({});
  /** @type {import('vue').Ref<Object|null>} */
  const doctor = ref(null);
  /** @type {import('vue').Ref<CapabilityCheckReport|null>} */
  const checkReport = ref(null);
  /** @type {import('vue').Ref<CapabilityMutationResult|null>} */
  const lastMutation = ref(null);
  /** @type {import('vue').Ref<Object|null>} */
  const lastReconcile = ref(null);
  /** @type {import('vue').Ref<string|null>} */
  const lastError = ref(null);
  const loading = ref(false);
  const checking = ref(false);
  const applying = ref(false);
  const reconciling = ref(false);

  const supportedTargets = computed(() => targets.value.filter(target => target.supported));

  /** Targets the user may select, defaulting to the ChatSpeed directory only. */
  const defaultSelection = computed(() => defaultTargetSelection(targets.value));

  /** Skills that may be removed right now. */
  const uninstallable = computed(() => skills.value.filter(skill => skill.uninstallable));

  async function loadInventory() {
    loading.value = true;
    lastError.value = null;
    try {
      // One call returns both lists so the page cannot show two scans.
      const inventory = await invokeCapability('capability_skill_inventory');
      targets.value = inventory?.targets ?? [];
      skills.value = inventory?.skills ?? [];
      return inventory;
    } catch (error) {
      lastError.value = error.message;
      throw error;
    } finally {
      loading.value = false;
    }
  }

  /**
   * Loads the MCP desired/runtime/tools projection.
   *
   * Kept separate from the legacy MCP list call so the settings page can show
   * the same facts the CLI reports without changing the edit wire (AC-12).
   */
  async function loadMcpServers() {
    lastError.value = null;
    try {
      const servers = await invokeCapability('capability_mcp_servers');
      mcpViews.value = mcpViewIndex(servers);
      return servers;
    } catch (error) {
      lastError.value = error.message;
      throw error;
    }
  }

  async function loadDoctor() {
    lastError.value = null;
    try {
      doctor.value = await invokeCapability('capability_doctor');
      return doctor.value;
    } catch (error) {
      lastError.value = error.message;
      throw error;
    }
  }

  /** Runs the non-LLM checker; it never writes into a target. */
  async function checkSource(source) {
    checking.value = true;
    lastError.value = null;
    try {
      checkReport.value = await invokeCapability('capability_skill_check', { source });
      return checkReport.value;
    } catch (error) {
      checkReport.value = null;
      lastError.value = error.message;
      throw error;
    } finally {
      checking.value = false;
    }
  }

  /**
   * Installs a checked source.
   *
   * @param {{source: Object, targets: string[], idempotencyKey?: string}} request
   */
  async function install({ source, targets: selection, idempotencyKey }) {
    applying.value = true;
    lastError.value = null;
    try {
      const key = idempotencyKey || newIdempotencyKey('skill-install');
      lastMutation.value = await invokeCapability('capability_skill_install', {
        source,
        targets: selection,
        idempotencyKey: key
      });
      await loadInventory();
      return lastMutation.value;
    } catch (error) {
      lastError.value = error.message;
      throw error;
    } finally {
      applying.value = false;
    }
  }

  /**
   * Uninstalls one managed Skill.
   *
   * @param {{skillName: string, targets: string[], idempotencyKey?: string}} request
   */
  async function uninstall({ skillName, targets: selection, idempotencyKey }) {
    applying.value = true;
    lastError.value = null;
    try {
      const key = idempotencyKey || newIdempotencyKey('skill-uninstall');
      lastMutation.value = await invokeCapability('capability_skill_uninstall', {
        skillName,
        targets: selection,
        idempotencyKey: key
      });
      await loadInventory();
      return lastMutation.value;
    } catch (error) {
      lastError.value = error.message;
      throw error;
    } finally {
      applying.value = false;
    }
  }

  /**
   * Converges capability drift the durable evidence proves.
   *
   * Delegates to the same capability service the CLI and HTTP adapters use, so
   * the desktop and `cs doctor reconcile` can never converge different facts
   * (AC-1). Only proven interrupted effects advance; anything unproven stays
   * `needs_reconcile`. Refreshes the inventory and the doctor report after.
   */
  async function reconcile() {
    reconciling.value = true;
    lastError.value = null;
    try {
      lastReconcile.value = await invokeCapability('capability_reconcile');
      await Promise.all([loadInventory(), loadDoctor()]);
      return lastReconcile.value;
    } catch (error) {
      lastError.value = error.message;
      throw error;
    } finally {
      reconciling.value = false;
    }
  }

  function resetCheck() {
    checkReport.value = null;
  }

  return {
    targets,
    skills,
    mcpViews,
    doctor,
    checkReport,
    lastMutation,
    lastReconcile,
    lastError,
    loading,
    checking,
    applying,
    reconciling,
    supportedTargets,
    defaultSelection,
    uninstallable,
    loadInventory,
    loadMcpServers,
    loadDoctor,
    checkSource,
    install,
    uninstall,
    reconcile,
    resetCheck
  };
});
