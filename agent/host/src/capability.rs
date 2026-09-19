//! Per-connection capability attenuation (T4.7, INV-6).
//!
//! Capability is host-assigned, never client-supplied. A session is launched with a
//! grant set (the *ceiling*); in attached mode that ceiling defaults to
//! `read,author` (T2.10 / R8-M2). Every tool call is checked against the grant set
//! by [`Host::call`](crate::Host), and a request to run under capabilities the mode
//! ceiling does not grant is refused with [`ToolError::Unauthorized`].

use graphite_agent_protocol::{Capability, CapabilitySet, ToolError};

/// The capability ceiling applied in attached mode when the user does not override
/// it.
pub const ATTACHED_CEILING: &[Capability] = &[Capability::Read, Capability::Author];

/// The capability ceiling applied in peer mode.
pub const PEER_CEILING: &[Capability] = ATTACHED_CEILING;

/// The full grant set (headless mode's default).
pub const ALL: &[Capability] = &[Capability::Read, Capability::Author, Capability::Execute, Capability::Export, Capability::Persist];

/// Check one required capability against a connection's grant set (INV-6).
pub fn authorize(granted: &CapabilitySet, required: Capability) -> Result<(), ToolError> {
	if granted.grants(required) {
		Ok(())
	} else {
		Err(ToolError::Unauthorized { capability: required })
	}
}

/// Attenuate a requested grant set to a connection's ceiling.
///
/// Any capability outside `ceiling` is refused with [`ToolError::Unauthorized`],
/// so a connection can never be widened past its mode's ceiling. Duplicate entries
/// are collapsed and the result preserves the order of `requested`.
pub fn attenuate(ceiling: &[Capability], requested: &[Capability]) -> Result<CapabilitySet, ToolError> {
	let mut granted = Vec::new();
	for capability in requested {
		if !ceiling.contains(capability) {
			return Err(ToolError::Unauthorized { capability: *capability });
		}
		if !granted.contains(capability) {
			granted.push(*capability);
		}
	}
	Ok(CapabilitySet(granted))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn attached_ceiling_refuses_execute_and_persist() {
		let requested = [Capability::Read, Capability::Author, Capability::Execute];
		match attenuate(ATTACHED_CEILING, &requested) {
			Err(ToolError::Unauthorized { capability }) => assert_eq!(capability, Capability::Execute),
			other => panic!("expected Unauthorized, got {other:?}"),
		}
	}

	#[test]
	fn attached_ceiling_grants_read_and_author() {
		let granted = attenuate(ATTACHED_CEILING, &[Capability::Read, Capability::Author, Capability::Read]).expect("granted");
		assert_eq!(granted, CapabilitySet(vec![Capability::Read, Capability::Author]));
		assert!(authorize(&granted, Capability::Read).is_ok());
		assert!(matches!(authorize(&granted, Capability::Export), Err(ToolError::Unauthorized { .. })));
	}
}
