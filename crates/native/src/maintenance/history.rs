//! Durable enumeration runs. Scan completion is deliberately not backup completion.
use super::*;

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum HistoryAction {
    Start,
    Pause,
    Resume,
}
#[derive(Serialize, Deserialize, Debug)]
pub struct HistoryStatus {
    pub run: i64,
    pub state: String,
    pub checked: i64,
    pub pending: i64,
}
impl Maintenance {
    pub fn history_status(&self, receiver: &str) -> Result<Option<HistoryStatus>> {
        let run: Option<(i64, String)> = self
            .conn
            .query_row(
                "SELECT run,state FROM history_runs WHERE receiver=?1",
                [receiver],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(database)?;
        run.map(|(run,state)| {
            let checked = self.conn.query_row("SELECT COUNT(*) FROM history_members WHERE receiver=?1", [receiver], |r|r.get(0)).map_err(database)?;
            let pending = self.conn.query_row("SELECT COUNT(*) FROM history_members h JOIN pending_sources p ON p.receiver=h.receiver AND p.source=h.source WHERE h.receiver=?1", [receiver], |r|r.get(0)).map_err(database)?;
            Ok(HistoryStatus {run,state,checked,pending})
        }).transpose()
    }
    pub fn history_control(&self, receiver: &str, action: HistoryAction) -> Result<HistoryStatus> {
        if receiver.is_empty() || receiver.len() > 256 {
            return Err(Error::Invalid("history receiver".into()));
        }
        let event = match &action {
            HistoryAction::Start => "history_scan_started",
            HistoryAction::Pause => "history_scan_paused",
            HistoryAction::Resume => "history_scan_resumed",
        };
        let tx = self.conn.unchecked_transaction().map_err(database)?;
        match action {
            HistoryAction::Start => {
                let old: Option<(i64, String)> = tx
                    .query_row(
                        "SELECT run,state FROM history_runs WHERE receiver=?1",
                        [receiver],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()
                    .map_err(database)?;
                // Repeated clicks are idempotent; starting again is explicit after enumeration.
                if old.as_ref().is_none_or(|(_, s)| s == "scanned") {
                    let run = old.map_or(1, |(r, _)| r.saturating_add(1));
                    tx.execute("INSERT INTO history_runs VALUES(?1,?2,'scanning') ON CONFLICT(receiver) DO UPDATE SET run=excluded.run,state=excluded.state",params![receiver,run]).map_err(database)?;
                    tx.execute("DELETE FROM history_members WHERE receiver=?1", [receiver])
                        .map_err(database)?;
                }
            }
            HistoryAction::Pause => {
                tx.execute(
                    "UPDATE history_runs SET state='paused' WHERE receiver=?1 AND state='scanning'",
                    [receiver],
                )
                .map_err(database)?;
            }
            HistoryAction::Resume => {
                tx.execute(
                    "UPDATE history_runs SET state='scanning' WHERE receiver=?1 AND state='paused'",
                    [receiver],
                )
                .map_err(database)?;
            }
        }
        tx.commit().map_err(database)?;
        let _ = self.log(event, None, None);
        self.history_status(receiver)?.ok_or(Error::NotFound)
    }
    pub fn history_batch(
        &self,
        receiver: &str,
        run: i64,
        sources: &[(String, String)],
        known: &BTreeMap<String, String>,
        finished: bool,
    ) -> Result<HistoryStatus> {
        if sources.len() > 200
            || sources.iter().any(|(id, rev)| {
                id.is_empty() || id.len() > 8192 || rev.is_empty() || rev.len() > 256
            })
        {
            return Err(Error::Invalid("history batch".into()));
        }
        let tx = self.conn.unchecked_transaction().map_err(database)?;
        let active: bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM history_runs WHERE receiver=?1 AND run=?2 AND state='scanning')",params![receiver,run],|r|r.get(0)).map_err(database)?;
        if !active {
            return Err(Error::Conflict("history scan changed".into()));
        }
        for (source, revision) in sources {
            tx.execute("INSERT INTO history_members VALUES(?1,?2,?3) ON CONFLICT(receiver,source) DO UPDATE SET revision=excluded.revision",params![receiver,source,revision]).map_err(database)?;
            if known.get(source).is_none_or(|state| state == "failed") {
                tx.execute(
                    "INSERT OR IGNORE INTO pending_sources(receiver,source) VALUES(?1,?2)",
                    params![receiver, source],
                )
                .map_err(database)?;
            }
        }
        if finished {
            tx.execute(
                "UPDATE history_runs SET state='scanned' WHERE receiver=?1 AND run=?2",
                params![receiver, run],
            )
            .map_err(database)?;
        }
        tx.commit().map_err(database)?;
        if finished {
            let _ = self.log("history_scan_finished", None, None);
        }
        self.history_status(receiver)?.ok_or(Error::NotFound)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn history_is_resumable_idempotent_and_receiver_scoped() {
        let root = std::env::temp_dir().join(format!("photobridge-history-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let m = Maintenance::open(&root).unwrap();
        m.schedule_sources("a", &["new-photo".into()]).unwrap();
        let first = m.history_control("a", HistoryAction::Start).unwrap();
        assert_eq!(
            m.history_control("a", HistoryAction::Start).unwrap().run,
            first.run
        );
        let sources = vec![
            ("one".into(), "1".into()),
            ("already-received".into(), "1".into()),
        ];
        let known = BTreeMap::from([("already-received".into(), "received".into())]);
        let s = m
            .history_batch("a", first.run, &sources, &known, false)
            .unwrap();
        assert_eq!((s.checked, s.pending), (2, 1));
        m.history_control("a", HistoryAction::Pause).unwrap();
        assert!(m
            .history_batch("a", first.run, &sources, &known, true)
            .is_err());
        assert_eq!(m.pending("a").unwrap()["count"], 2);
        drop(m);
        let m = Maintenance::open(&root).unwrap();
        assert_eq!(m.history_status("a").unwrap().unwrap().state, "paused");
        assert!(m.history_status("b").unwrap().is_none());
        m.history_control("a", HistoryAction::Resume).unwrap();
        m.source_result("a", "one", true).unwrap();
        let known = BTreeMap::from([
            ("one".into(), "queued".into()),
            ("already-received".into(), "received".into()),
        ]);
        let s = m
            .history_batch("a", first.run, &sources, &known, true)
            .unwrap();
        assert_eq!((s.checked, s.pending), (2, 0));
        assert_eq!(s.state, "scanned");
        assert_eq!(m.pending("a").unwrap()["count"], 1); // incremental work survives
        let second = m.history_control("a", HistoryAction::Start).unwrap();
        assert!(second.run > first.run);
        assert!(m
            .history_batch("a", first.run, &sources, &known, true)
            .is_err());
        m.history_batch("a", second.run, &sources, &known, true)
            .unwrap();
        assert_eq!(m.pending("a").unwrap()["count"], 1);
        drop(m);
        fs::remove_dir_all(root).unwrap();
    }
}
