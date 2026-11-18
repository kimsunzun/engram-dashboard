// ADR-0064 §4 invariant (no scattered imports): new content = its module + one line here.
//
// ★Mind the load order★: slotCommands / slotContentCommands / agentCommands register their commands
//   and *then* reference those ids with registerSlotMenu. buildSlotMenu's resolve happens at render time (right-click),
//   so it does not depend on import order, but every register must be done by manifest load time for right-click to be safe
//   — this file imports them all, so that invariant holds.

import './tabCommands'
import './slotCommands'
import './slotContentCommands'
import './renderModeCommands'
import './agentCommands'
import './scrollCommands'
import './chatCommands'
import './usageCommands'
import './connectionCommands'

import { validateSlotMenuContributions } from './slotMenu'

// FIX-1 (ADR-0064): full check after every command and contribution module is imported (= register done). If a contributed commandId is
//   not in the registry, it is caught right at boot with console.error (without waiting for buildSlotMenu's skip+log at
//   right-click). fail-loud achieved without a crash — at this point every register is done by the side-effect imports above.
validateSlotMenuContributions()
