//! Generic burst relationship, independent of any receiver's presentation format.
use crate::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BurstMetadata {
    pub group_id: String,
    pub primary: bool,
}
impl BurstMetadata {
    pub fn from_identifier(identifier: &str, primary: bool) -> Result<Self> {
        if identifier.is_empty() || identifier.len() > 1024 {
            return Err(Error::Invalid("burst identifier".into()));
        }
        Ok(Self {
            group_id: digest(format!("photobridge-burst-v1:{identifier}").as_bytes()),
            primary,
        })
    }
    pub fn validate(&self) -> Result<()> {
        if !valid_digest(&self.group_id) {
            return Err(Error::Invalid("burst group".into()));
        }
        Ok(())
    }
    pub fn fields(&self) -> BTreeMap<String, String> {
        BTreeMap::from([
            ("burst_group_ref".into(), self.group_id.clone()),
            ("burst_primary".into(), self.primary.to_string()),
            ("burst_metadata_version".into(), "1".into()),
        ])
    }
    pub fn from_fields(fields: &BTreeMap<String, String>) -> Result<Option<Self>> {
        if !["burst_group_ref", "burst_primary", "burst_metadata_version"]
            .iter()
            .any(|k| fields.contains_key(*k))
        {
            return Ok(None);
        }
        let group_id = fields
            .get("burst_group_ref")
            .ok_or_else(|| Error::Invalid("burst group".into()))?
            .clone();
        let primary = match fields.get("burst_primary").map(String::as_str) {
            Some("true") => true,
            Some("false") => false,
            _ => return Err(Error::Invalid("burst primary".into())),
        };
        if fields.get("burst_metadata_version").map(String::as_str) != Some("1") {
            return Err(Error::Unsupported("burst metadata version".into()));
        }
        let burst = Self { group_id, primary };
        burst.validate()?;
        Ok(Some(burst))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn burst_relationship_is_stable_and_strict() {
        let primary = BurstMetadata::from_identifier("native-group", true).unwrap();
        let sibling = BurstMetadata::from_identifier("native-group", false).unwrap();
        assert_eq!(primary.group_id, sibling.group_id);
        assert!(!primary.group_id.contains("native-group"));
        assert_eq!(
            BurstMetadata::from_fields(&primary.fields()).unwrap(),
            Some(primary.clone())
        );
        assert!(BurstMetadata::from_fields(&BTreeMap::new())
            .unwrap()
            .is_none());
        let mut invalid = primary.fields();
        invalid.remove("burst_primary");
        assert!(BurstMetadata::from_fields(&invalid).is_err());
        invalid.insert("burst_primary".into(), "true".into());
        invalid.insert("burst_group_ref".into(), "<xml/>".into());
        assert!(BurstMetadata::from_fields(&invalid).is_err());
    }
}
