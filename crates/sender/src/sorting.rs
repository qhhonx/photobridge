use super::*;

/// Filters, ordering and continuation cursor for one page of transfer jobs.
#[derive(Clone, Copy, Debug)]
pub struct JobQuery<'a> {
    pub after: i64,
    pub after_value: Option<i64>,
    pub limit: u32,
    pub receiver: Option<&'a str>,
    pub state: Option<&'a str>,
    pub sort: &'a str,
    pub descending: bool,
}

pub(super) fn migrate(conn: &Connection) -> Result<()> {
    let present: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('jobs') WHERE name='state_changed_at_ms')", [], |r|r.get(0)).map_err(db)?;
    if !present {
        conn.execute(
            "ALTER TABLE jobs ADD COLUMN state_changed_at_ms INTEGER",
            [],
        )
        .map_err(db)?;
    }
    // Old rows stay unknown: opening the app is not their completion/failure time.
    conn.execute_batch("CREATE TRIGGER IF NOT EXISTS jobs_state_time AFTER UPDATE OF state ON jobs WHEN old.state != new.state BEGIN UPDATE jobs SET state_changed_at_ms=CAST(strftime('%s','now') AS INTEGER)*1000 WHERE id=new.id; END;
        CREATE INDEX IF NOT EXISTS jobs_state_time_order ON jobs(receiver_id,state,state_changed_at_ms,id);
        CREATE INDEX IF NOT EXISTS jobs_capture_time_order ON jobs(receiver_id,state,CAST(json_extract(manifest,'$.metadata.created_at_ms') AS INTEGER),id);").map_err(db)?;
    Ok(())
}

impl Sender {
    /// Cursor contains both the ordering value and the ID; ties and a moving
    /// anchor do not reset pagination. Unknown dates always sort last.
    pub fn browse(&self, query: JobQuery<'_>) -> Result<Vec<Job>> {
        let JobQuery {
            after,
            after_value,
            limit,
            receiver,
            state,
            sort,
            descending,
        } = query;
        let expression = match sort {
            "added" => "id",
            "capture" => "CAST(json_extract(manifest,'$.metadata.created_at_ms') AS INTEGER)",
            "activity" => "state_changed_at_ms",
            "retry" => "next_attempt_at*1000",
            _ => return Err(Error::Invalid("task sort".into())),
        };
        let unknown = if descending { i64::MIN } else { i64::MAX };
        let value = format!("CASE WHEN {expression}>0 THEN {expression} ELSE {unknown} END");
        let compare = if descending { "<" } else { ">" };
        let direction = if descending { "DESC" } else { "ASC" };
        let cursor = if after == 0 {
            0
        } else {
            after_value.ok_or_else(|| Error::Invalid("sort cursor".into()))?
        };
        let sql = format!("SELECT id,{value} FROM jobs WHERE (?3 IS NULL OR receiver_id=?3) AND (?4 IS NULL OR state=?4) AND (?1=0 OR ({value},id){compare}(?5,?1)) ORDER BY {value} {direction},id {direction} LIMIT ?2");
        let mut stmt = self.conn.prepare(&sql).map_err(db)?;
        let rows = stmt
            .query_map(
                params![after, limit.clamp(1, 500), receiver, state, cursor],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
            )
            .map_err(db)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db)?;
        rows.into_iter()
            .map(|(id, value)| {
                let mut job = self.job(id)?;
                job.sort_value = Some(value);
                Ok(job)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};
    #[test]
    fn date_order_paginates_ties_unknowns_and_state_changes() {
        let path = std::env::temp_dir().join(format!(
            "photobridge-sort-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut s = Sender::open(&path).unwrap();
        let manifest = r#"{"version":1,"source_id":"source","revision":"1","kind":"photo","metadata":{},"resources":[]}"#;
        for id in 1..=207 {
            let mut asset: serde_json::Value = serde_json::from_str(manifest).unwrap();
            if id > 2 {
                asset["metadata"]["created_at_ms"] = json_string((id / 3) * 1000);
            }
            s.conn.execute("INSERT INTO jobs(id,receiver_id,asset_id,manifest,sources,state) VALUES(?1,?2,?3,?4,'{}',?5)",params![id,if id==207 {"other"} else {"r"},id.to_string(),asset.to_string(),if id==206 {"failed"} else {"received"}]).unwrap();
        }
        for descending in [false, true] {
            let mut ids = Vec::new();
            let mut cursor = 0;
            let mut value = None;
            loop {
                let page = s
                    .browse(JobQuery {
                        after: cursor,
                        after_value: value,
                        limit: 70,
                        receiver: Some("r"),
                        state: Some("received"),
                        sort: "capture",
                        descending,
                    })
                    .unwrap();
                if page.is_empty() {
                    break;
                }
                cursor = page.last().unwrap().id;
                value = page.last().unwrap().sort_value;
                ids.extend(page.iter().map(|j| j.id));
            }
            assert_eq!(ids.len(), 205);
            assert_eq!(ids.iter().copied().collect::<BTreeSet<_>>().len(), 205);
            assert!(ids[203..].iter().all(|id| *id <= 2));
            assert!(ids[..203].windows(2).all(|p| if descending {
                p[0] > p[1]
            } else {
                p[0] < p[1]
            }));
        }
        assert!(s.job(206).unwrap().state_changed_at_ms.is_none());
        s.conn
            .execute("UPDATE jobs SET state='running' WHERE id=206", [])
            .unwrap();
        assert!(s.job(206).unwrap().state_changed_at_ms.unwrap() > 0);
        let failed = s.job(206).unwrap();
        s.fail(&failed.attempt(), Failure::Network, 100).unwrap();
        let query = JobQuery {
            after: 0,
            after_value: None,
            limit: 10,
            receiver: Some("r"),
            state: Some("waiting"),
            sort: "retry",
            descending: false,
        };
        assert_eq!(s.browse(query).unwrap()[0].id, 206);
        assert!(s
            .browse(JobQuery {
                after: 1,
                receiver: None,
                state: None,
                sort: "capture",
                ..query
            })
            .is_err());
        assert!(s
            .browse(JobQuery {
                receiver: None,
                state: None,
                sort: "untrusted SQL",
                ..query
            })
            .is_err());
        // Direction/state filters are applied before pagination; legacy ID order is unchanged.
        assert_eq!(s.list_filtered(0, 1, Some("r"), None).unwrap()[0].id, 1);
        drop(s);
        let _ = fs::remove_dir_all(path);
    }
    fn json_string(n: i64) -> serde_json::Value {
        serde_json::Value::String(n.to_string())
    }
}
