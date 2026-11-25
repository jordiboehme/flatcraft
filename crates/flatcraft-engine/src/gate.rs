//! The send gate: may this origin start a job on this transport?

use flatcraft_device::TransportSpec;

use crate::prefs::Preferences;
use crate::session::Origin;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateDecision {
    Allow,
    /// The person at the FlatCraft window has to confirm the send.
    NeedsConfirmation,
    Refuse(String),
}

/// Text of the headless refusal.
pub const HEADLESS_REFUSAL: &str = "Agents may not start USB cuts in a headless session. Turn on 'Allow agents to start cuts' in the preferences.";

/// File Output is always allowed. A USB cut is allowed for the person, and
/// for an agent only with the preference `allow_agent_cuts`. Without it an
/// agent needs the person to confirm in the window, and a headless session
/// refuses.
pub fn check_send(
    origin: Origin,
    transport: &TransportSpec,
    prefs: &Preferences,
    has_ui: bool,
) -> GateDecision {
    match transport {
        TransportSpec::File { .. } => GateDecision::Allow,
        TransportSpec::Usb { .. } => match origin {
            Origin::User => GateDecision::Allow,
            Origin::Agent if prefs.allow_agent_cuts => GateDecision::Allow,
            Origin::Agent if has_ui => GateDecision::NeedsConfirmation,
            Origin::Agent => GateDecision::Refuse(HEADLESS_REFUSAL.into()),
        },
    }
}
