use serde::{Deserialize, Serialize};

/// Stable protocol behavior implemented by a BearWire client.
///
/// Wire names are permanent compatibility contracts: never reuse a name or
/// change its meaning. Additive behavior gets a new capability; an incompatible
/// semantic change gets a new capability or protocol generation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtocolCapability {
    ToolAttemptToken,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CompatibilityManifest {
    pub protocol: u32,
    pub capabilities: Vec<ProtocolCapability>,
}

impl CompatibilityManifest {
    pub fn armature() -> Self {
        Self {
            protocol: 1,
            capabilities: vec![ProtocolCapability::ToolAttemptToken],
        }
    }

    pub fn missing<'a>(
        &'a self,
        required: &'a [ProtocolCapability],
    ) -> impl Iterator<Item = ProtocolCapability> + 'a {
        required
            .iter()
            .copied()
            .filter(|capability| !self.capabilities.contains(capability))
    }
}

/// Capabilities Den cannot safely execute a work run without.
///
/// Adding an item here makes older sandbox images ineligible. Only require a
/// capability when Den cannot safely fall back; merely supporting new behavior
/// is not enough.
pub const REQUIRED_WORK_CAPABILITIES: &[ProtocolCapability] =
    &[ProtocolCapability::ToolAttemptToken];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_reports_missing_capabilities() {
        let manifest = CompatibilityManifest {
            protocol: 1,
            capabilities: vec![],
        };

        assert_eq!(
            manifest
                .missing(REQUIRED_WORK_CAPABILITIES)
                .collect::<Vec<_>>(),
            vec![ProtocolCapability::ToolAttemptToken]
        );
    }
}
