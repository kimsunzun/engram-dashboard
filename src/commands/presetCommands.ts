// ADR-0061 / ADR-0055: only routes to the agentClient preset methods (ProtocolClient seam, ADR-0011) — zero new state paths; the store update is the PresetListUpdated broadcast's job.
// ★ADR-0064 slot menu contribution co-location★: content (preset) knowledge lives in one module (the common part is owned by '*').

import { open } from '@tauri-apps/plugin-dialog'

import { t } from '../i18n'
import { agentClient } from '../api/clientFactory'
import { register } from './registry'
import { registerSlotMenu } from './slotMenu'

register({
  id: 'preset.list',
  title: () => t('preset.list'),
  category: 'preset',
  // Does not replace the store mirror (a query surface).
  run: () => agentClient.listPresets(),
})

register({
  id: 'preset.create',
  title: () => t('preset.create'),
  category: 'preset',
  // The name is not passed — the backend stores only {id,cwd} and the frontend derives the display name from the cwd basename (ADR-0061).
  run: (args) => {
    const cwd = args?.cwd as string | undefined
    if (!cwd || !cwd.trim()) {
      throw new Error(`[preset.create] cwd is empty: ${String(cwd)}`)
    }
    return agentClient.createPreset(cwd.trim())
  },
})

register({
  id: 'preset.delete',
  title: () => t('preset.delete'),
  category: 'preset',
  // An unknown id is a backend no-op (Ack) — the frontend does not check existence (ADR-0061).
  run: (args) => {
    const id = args?.id as string | undefined
    if (!id || !id.trim()) {
      throw new Error(`[preset.delete] id is empty: ${String(id)}`)
    }
    return agentClient.deletePreset(id.trim())
  },
})

register({
  id: 'preset.rename',
  title: () => t('preset.rename'),
  category: 'preset',
  // ADR-0061 enrichment (§5 LLM control): set/clear the preset display-name override — name string → store it; null/empty/unspecified → clear it (back to the cwd basename derivation). Reflected through the PresetListUpdated broadcast (no optimistic update); an unknown id is a backend no-op (Ack). Rename is a backend save rather than frontend-only so the LLM changes the display name through the same command surface.
  run: (args) => {
    const id = args?.id as string | undefined
    if (!id || !id.trim()) {
      throw new Error(`[preset.rename] id is empty: ${String(id)}`)
    }
    const raw = args?.name
    const name = typeof raw === 'string' && raw.trim().length > 0 ? raw.trim() : null
    return agentClient.renamePreset(id.trim(), name)
  },
})

register({
  id: 'preset.add',
  title: () => t('preset.add'),
  category: 'preset',
  // ★Native folder dialog → createPreset★ (ADR-0061 / ADR-0064): the old PresetPalette pane "Add" menu logic promoted to a command; the store update is the PresetListUpdated broadcast (no optimistic update). The dialog is a native window outside the webview, so cdp cannot check it — only the open→createPreset wiring is unit tested.
  run: async () => {
    const picked = await open({ directory: true, multiple: false, title: t('dialog.pickPresetPath') })
    const cwd = typeof picked === 'string' ? picked : null
    if (!cwd) return // cancelled — no-op
    return agentClient.createPreset(cwd)
  },
})

// ADR-0064: group='content' — rendered above the common slot-ops.
registerSlotMenu('preset_palette', [{ commandId: 'preset.add', group: 'content', order: 10 }])
