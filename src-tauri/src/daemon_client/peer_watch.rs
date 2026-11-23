//! The shell's peer watch — what it tells the daemon about the PC list slot: `subscribed` = some window has a PC list slot in any tab
//!   (active or not; the window focused, behind others, minimized or hidden to the tray). The daemon then sends the peer state and
//!   routes approval questions to the app instead of the OS dialog (ADR-0410 — a question buried behind other windows is the user's to
//!   find). Detach is the connection's own end: the daemon reroutes when the connection drops.
//!
//! ★No delay on losing it★: the layout model never passes through a state without the slot while one moves — a slot moved to another
//!   window stays at its source until the lock that attaches it there also closes the source (`layout::apply::move_slot_to_window`), and
//!   a tab moved to a new window is attached and removed in one lock — so a recompute cannot flap the route.
//!
//! Pure — knows nothing of Tauri, sockets or clocks: [`PeerWatch::layout_changed`] returns what to send; the caller sends it
//!   (`super::DaemonClient::peer_layout_changed`). Lock order: `ViewManager → this`.

// ADR-0394, ADR-0410

use crate::layout::{LayoutNode, SlotContent, ViewManager};

/// What the daemon is told — the field of `AgentCommand::PeerSubscribe`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PeerWant {
    pub subscribed: bool,
}

/// One shell's peer watch (module head).
#[derive(Debug, Default)]
pub struct PeerWatch {
    /// What was last handed out to be sent.
    sent: PeerWant,
}

/// Whether `node` holds a PC list slot.
fn has_peers(node: &LayoutNode) -> bool {
    match node {
        LayoutNode::Slot {
            content: SlotContent::Peers,
            ..
        } => true,
        LayoutNode::Slot { .. } => false,
        LayoutNode::Split { a, b, .. } => has_peers(a) || has_peers(b),
    }
}

/// What the layout wants (module head).
pub fn peer_want(mgr: &ViewManager) -> PeerWant {
    let subscribed = mgr.windows.values().any(|tabs| {
        tabs.tabs
            .iter()
            .any(|view| mgr.views.get(view).is_some_and(|v| has_peers(&v.layout)))
    });
    PeerWant { subscribed }
}

impl PeerWatch {
    pub fn new() -> Self {
        Self::default()
    }

    /// The layout changed (inside the ViewManager lock) — `Some` = tell the daemon this now.
    pub fn layout_changed(&mut self, mgr: &ViewManager) -> Option<PeerWant> {
        let wanted = peer_want(mgr);
        (wanted != self.sent).then(|| {
            self.sent = wanted;
            wanted
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{tree, MAIN_WINDOW_LABEL};

    fn mgr_with(content: SlotContent) -> ViewManager {
        let mut mgr = ViewManager::new();
        let view = mgr.windows[MAIN_WINDOW_LABEL].active;
        let slot = tree::first_slot_id(&mgr.views[&view].layout);
        mgr.set_slot_content(view, slot, content)
            .expect("the main tab's slot takes content");
        mgr
    }

    #[test]
    fn a_peers_slot_subscribes_and_other_content_does_not() {
        assert_eq!(
            peer_want(&mgr_with(SlotContent::Peers)),
            PeerWant { subscribed: true }
        );
        assert_eq!(
            peer_want(&mgr_with(SlotContent::AgentList)),
            PeerWant::default()
        );
    }

    /// ADR-0410: a PC list slot in a tab that is not the active one still counts — on screen or not.
    #[test]
    fn a_peers_slot_in_an_inactive_tab_still_subscribes() {
        let mut mgr = mgr_with(SlotContent::Peers);
        let peers_tab = mgr.windows[MAIN_WINDOW_LABEL].active;
        mgr.create_tab(MAIN_WINDOW_LABEL, None)
            .expect("main takes a new tab");
        assert_ne!(mgr.windows[MAIN_WINDOW_LABEL].active, peers_tab);
        assert_eq!(peer_want(&mgr), PeerWant { subscribed: true });
    }

    #[test]
    fn only_a_change_is_sent_and_closing_the_slot_is_sent_at_once() {
        let mut watch = PeerWatch::new();
        assert_eq!(
            watch.layout_changed(&mgr_with(SlotContent::AgentList)),
            None
        );
        let with = mgr_with(SlotContent::Peers);
        assert_eq!(
            watch.layout_changed(&with),
            Some(PeerWant { subscribed: true })
        );
        assert_eq!(watch.layout_changed(&with), None, "no change, nothing sent");
        assert_eq!(
            watch.layout_changed(&mgr_with(SlotContent::AgentList)),
            Some(PeerWant::default())
        );
    }
}
