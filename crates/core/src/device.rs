use crate::*;

/// Display identity only. It is never used as a credential or asset identifier.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceProfile {
    pub id: String,
    pub name: String,
}
impl DeviceProfile {
    pub fn validate(&self) -> Result<()> {
        if !valid_digest(&self.id)
            || self.name.trim() != self.name
            || self.name.is_empty()
            || self.name.chars().count() > 40
            || self.name.len() > 160
            || self.name.chars().any(|c| {
                c.is_control()
                    || matches!(c,
                '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
            })
        {
            return Err(Error::Invalid("device profile".into()));
        }
        Ok(())
    }
}
