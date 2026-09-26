//! Bounded, metadata-only folder indexing. No PhotoKit, UI or receiver knowledge.
//! Sources are read-only; missing files never imply remote deletion.
use photobridge_core::{digest, Error, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::{
    fs,
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

fn db(e: rusqlite::Error) -> Error {
    Error::Storage(e.to_string())
}
#[derive(Debug, Serialize)]
pub struct Entry {
    pub relative: String,
    pub source_id: String,
    pub revision: String,
    pub size: u64,
    pub media_type: String,
    pub modified_ms: u64,
    pub job_id: Option<i64>,
}
#[derive(Debug, Serialize)]
pub struct Summary {
    pub files: u64,
    pub bytes: u64,
    pub unsupported: u64,
    pub scanning: bool,
}
struct Scan {
    source: String,
    root: PathBuf,
    generation: u64,
    scopes: Vec<String>,
    current: Option<fs::ReadDir>,
    unsupported: u64,
    files: u64,
    bytes: u64,
}
pub struct Index {
    conn: Connection,
    scan: Option<Scan>,
}
impl Index {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path).map_err(db)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;
          CREATE TABLE IF NOT EXISTS scan_dirs(source TEXT NOT NULL,path TEXT NOT NULL,PRIMARY KEY(source,path));
          CREATE TABLE IF NOT EXISTS files(source TEXT NOT NULL,relative TEXT NOT NULL,identity TEXT NOT NULL,revision TEXT NOT NULL,size INTEGER NOT NULL,mime TEXT NOT NULL,modified INTEGER NOT NULL,observed INTEGER NOT NULL,generation INTEGER NOT NULL,present INTEGER NOT NULL DEFAULT 1,PRIMARY KEY(source,relative));
          CREATE INDEX IF NOT EXISTS folder_identity ON files(source,identity);
          CREATE INDEX IF NOT EXISTS folder_candidates ON files(source,present,modified);
          CREATE TABLE IF NOT EXISTS ignored(source TEXT NOT NULL,relative TEXT NOT NULL,revision TEXT NOT NULL,until_ms INTEGER NOT NULL DEFAULT 0,PRIMARY KEY(source,relative));
          CREATE TABLE IF NOT EXISTS submitted(source TEXT NOT NULL,identity TEXT NOT NULL,receiver TEXT NOT NULL,revision TEXT NOT NULL,job INTEGER NOT NULL,PRIMARY KEY(source,identity,receiver));").map_err(db)?;
        Ok(Self { conn, scan: None })
    }
    pub fn begin(&mut self, source: &str, root: &Path, generation: u64) -> Result<()> {
        self.begin_scoped(source, root, generation, &[])
    }
    pub fn begin_scoped(
        &mut self,
        source: &str,
        root: &Path,
        generation: u64,
        scopes: &[String],
    ) -> Result<()> {
        if source.is_empty()
            || source.len() > 128
            || !root.is_dir()
            || fs::symlink_metadata(root)?.file_type().is_symlink()
        {
            return Err(Error::Invalid("folder source".into()));
        }
        if self.scan.is_some() {
            return Err(Error::Conflict("folder scan running".into()));
        }
        let mut scopes = if scopes.is_empty() {
            vec![String::new()]
        } else {
            scopes.to_vec()
        };
        scopes.sort();
        scopes.dedup();
        let mut filtered: Vec<String> = Vec::new();
        for scope in scopes {
            if !filtered
                .iter()
                .any(|p| p.is_empty() || scope.starts_with(&(p.clone() + "/")))
            {
                filtered.push(scope);
            }
        }
        let scopes = filtered;
        for relative in &scopes {
            if Path::new(relative)
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
            {
                return Err(Error::Invalid("folder scope".into()));
            }
        }
        let canonical = root.canonicalize()?;
        if scopes.len() > 512 {
            return Err(Error::Invalid("folder scopes".into()));
        }
        self.conn.execute("DELETE FROM scan_dirs", []).map_err(db)?;
        for relative in &scopes {
            let path = canonical.join(relative);
            if path.exists() {
                if !path.is_dir()
                    || fs::symlink_metadata(&path)?.file_type().is_symlink()
                    || path.canonicalize()? != path
                {
                    return Err(Error::Invalid("folder scope link".into()));
                }
                self.conn
                    .execute(
                        "INSERT OR IGNORE INTO scan_dirs(source,path) VALUES(?1,?2)",
                        params![source, relative],
                    )
                    .map_err(db)?;
            }
        }
        self.scan = Some(Scan {
            source: source.into(),
            root: canonical,
            generation,
            scopes,
            current: None,
            unsupported: 0,
            files: 0,
            bytes: 0,
        });
        Ok(())
    }
    pub fn cancel(&mut self) {
        let _ = self.conn.execute("DELETE FROM scan_dirs", []);
        self.scan = None;
    }
    pub fn step(&mut self, observed: u64, limit: usize) -> Result<Summary> {
        self.conn.execute_batch("BEGIN IMMEDIATE").map_err(db)?;
        let result = self.step_inner(observed, limit.clamp(1, 500));
        if result.is_ok() {
            self.conn.execute_batch("COMMIT").map_err(db)?;
        } else {
            let _ = self.conn.execute_batch("ROLLBACK");
        }
        if result.is_err() {
            self.scan = None;
        } // Incomplete scan never marks unseen files missing.
        result
    }
    fn step_inner(&mut self, observed: u64, limit: usize) -> Result<Summary> {
        let scan = self.scan.as_mut().ok_or(Error::NotFound)?;
        for _ in 0..limit {
            if scan.current.is_none() {
                let dir: Option<String> = self
                    .conn
                    .query_row(
                        "SELECT path FROM scan_dirs WHERE source=?1 ORDER BY rowid LIMIT 1",
                        [&scan.source],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(db)?;
                let Some(dir) = dir else {
                    for prefix in &scan.scopes {
                        self.conn.execute("UPDATE files SET present=0 WHERE source=?1 AND generation<>?2 AND (?3='' OR substr(relative,1,length(?3)+1)=?3||'/')",params![scan.source,scan.generation as i64,prefix]).map_err(db)?;
                    }
                    let source = scan.source.clone();
                    let unsupported = scan.unsupported;
                    self.scan = None;
                    return self.summary(&source, unsupported);
                };
                self.conn
                    .execute(
                        "DELETE FROM scan_dirs WHERE source=?1 AND path=?2",
                        params![scan.source, dir],
                    )
                    .map_err(db)?;
                let path = scan.root.join(dir);
                if fs::symlink_metadata(&path)?.file_type().is_symlink()
                    || path.canonicalize()? != path
                {
                    return Err(Error::Invalid("folder directory changed".into()));
                }
                scan.current = Some(fs::read_dir(path)?);
            }
            let Some(entry) = scan.current.as_mut().unwrap().next() else {
                scan.current = None;
                continue;
            };
            let entry = entry?;
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') {
                continue;
            }
            let typ = entry.file_type()?;
            if typ.is_symlink() {
                continue;
            }
            let path = entry.path();
            let relative = path
                .strip_prefix(&scan.root)
                .map_err(|_| Error::Invalid("folder containment".into()))?;
            let relative = relative
                .components()
                .map(|p| {
                    p.as_os_str()
                        .to_str()
                        .ok_or_else(|| Error::Unsupported("non-UTF8 filename".into()))
                })
                .collect::<Result<Vec<_>>>()?
                .join("/");
            if typ.is_dir() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    if entry.metadata()?.dev() != fs::metadata(&scan.root)?.dev() {
                        continue;
                    }
                }
                self.conn
                    .execute(
                        "INSERT OR IGNORE INTO scan_dirs(source,path) VALUES(?1,?2)",
                        params![scan.source, relative],
                    )
                    .map_err(db)?;
                continue;
            }
            if !typ.is_file() {
                continue;
            }
            let Some(mime) = media_type(&path) else {
                scan.unsupported += 1;
                continue;
            };
            let meta = entry.metadata()?;
            if meta.len() == 0 {
                continue;
            }
            scan.files += 1;
            scan.bytes += meta.len();
            let modified = millis(meta.modified()?);
            let revision = format!(
                "{}:{}",
                meta.len(),
                meta.modified()?
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            );

            #[cfg(unix)]
            let identity = {
                use std::os::unix::fs::MetadataExt;
                format!("{}", meta.ino())
            };
            #[cfg(not(unix))]
            let identity = relative.clone();
            self.conn.execute("INSERT INTO files(source,relative,identity,revision,size,mime,modified,observed,generation,present) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,1) ON CONFLICT(source,relative) DO UPDATE SET identity=excluded.identity,observed=CASE WHEN files.revision=excluded.revision AND files.identity=excluded.identity THEN files.observed ELSE excluded.observed END,revision=excluded.revision,size=excluded.size,mime=excluded.mime,modified=excluded.modified,generation=excluded.generation,present=1", params![scan.source,relative,identity,revision,meta.len() as i64,mime,modified as i64,observed as i64,scan.generation as i64]).map_err(db)?;
        }
        Ok(Summary {
            files: scan.files,
            bytes: scan.bytes,
            unsupported: scan.unsupported,
            scanning: true,
        })
    }
    pub fn summary(&self, source: &str, unsupported: u64) -> Result<Summary> {
        let (files, bytes) = self
            .conn
            .query_row(
                "SELECT COUNT(*),COALESCE(SUM(size),0) FROM files WHERE source=?1 AND present=1",
                [source],
                |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u64)),
            )
            .map_err(db)?;
        Ok(Summary {
            files,
            bytes,
            unsupported,
            scanning: self.scan.as_ref().is_some_and(|s| s.source == source),
        })
    }
    pub fn candidates(
        &self,
        source: &str,
        receiver: &str,
        now: u64,
        limit: usize,
    ) -> Result<Vec<Entry>> {
        let mut stmt=self.conn.prepare("SELECT relative,identity,revision,size,mime,modified FROM files f WHERE source=?1 AND present=1 AND observed<=?3 AND NOT EXISTS(SELECT 1 FROM ignored i WHERE i.source=f.source AND i.relative=f.relative AND i.revision=f.revision AND (i.until_ms=0 OR i.until_ms>?5)) AND NOT EXISTS(SELECT 1 FROM submitted s WHERE s.source=f.source AND s.identity=f.identity AND (s.receiver=?2 OR s.receiver='@baseline') AND s.revision=f.revision) ORDER BY modified DESC,relative LIMIT ?4").map_err(db)?;
        let rows = stmt
            .query_map(
                params![
                    source,
                    receiver,
                    now.saturating_sub(10_000) as i64,
                    limit.clamp(1, 200) as i64,
                    now as i64
                ],
                |r| {
                    let identity: String = r.get(1)?;
                    Ok(Entry {
                        relative: r.get(0)?,
                        source_id: asset_id(source, &identity),
                        revision: r.get(2)?,
                        size: r.get::<_, i64>(3)? as u64,
                        media_type: r.get(4)?,
                        modified_ms: r.get::<_, i64>(5)? as u64,
                        job_id: None,
                    })
                },
            )
            .map_err(db)?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(db)
    }
    pub fn mark(
        &self,
        source: &str,
        relative: &str,
        receiver: &str,
        revision: &str,
        job: i64,
    ) -> Result<()> {
        let changed=self.conn.execute("INSERT INTO submitted(source,identity,receiver,revision,job) SELECT source,identity,?3,revision,?5 FROM files WHERE source=?1 AND relative=?2 AND revision=?4 AND present=1 ON CONFLICT(source,identity,receiver) DO UPDATE SET revision=excluded.revision,job=excluded.job", params![source,relative,receiver,revision,job]).map_err(db)?;
        if changed == 0 {
            return Err(Error::Conflict("folder file changed".into()));
        }
        Ok(())
    }
}
pub fn asset_id(source: &str, identity: &str) -> String {
    format!("folder:{}:{}", source, digest(identity.as_bytes()))
}
pub fn millis(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}
pub fn media_type(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => Some("image/jpeg"),
        "heic" | "heif" => Some("image/heic"),
        "png" => Some("image/png"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "tif" | "tiff" => Some("image/tiff"),
        "mov" => Some("video/quicktime"),
        "mp4" | "m4v" => Some("video/mp4"),
        _ => None,
    }
}
/// Reject symbolic links even inside the selected root; never follow an escaped path.
pub fn resolve(root: &Path, relative: &str) -> Result<PathBuf> {
    let relative = Path::new(relative);
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(Error::Invalid("folder relative path".into()));
    }
    let root = root.canonicalize()?;
    let mut path = root.clone();
    for part in relative.components() {
        path.push(part);
        if fs::symlink_metadata(&path)?.file_type().is_symlink() {
            return Err(Error::Invalid("folder symlink".into()));
        }
    }
    if !path.is_file() || !path.canonicalize()?.starts_with(root) {
        return Err(Error::Invalid("folder containment".into()));
    }
    Ok(path)
}

pub fn revision(path: &Path) -> Result<String> {
    let m = fs::metadata(path)?;
    Ok(format!(
        "{}:{}",
        m.len(),
        m.modified()?
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ))
}
impl Index {
    pub fn page(&self, source: &str, offset: usize, receiver: &str) -> Result<Vec<Entry>> {
        let mut stmt=self.conn.prepare("SELECT relative,identity,revision,size,mime,modified,(SELECT job FROM submitted s WHERE s.source=f.source AND s.identity=f.identity AND (s.receiver=?3 OR s.receiver='@baseline') AND s.revision=f.revision ORDER BY s.job DESC LIMIT 1) FROM files f WHERE source=?1 AND present=1 ORDER BY modified DESC,relative LIMIT 100 OFFSET ?2").map_err(db)?;
        let rows = stmt
            .query_map(params![source, offset as i64, receiver], |r| {
                let identity: String = r.get(1)?;
                Ok(Entry {
                    relative: r.get(0)?,
                    source_id: asset_id(source, &identity),
                    revision: r.get(2)?,
                    size: r.get::<_, i64>(3)? as u64,
                    media_type: r.get(4)?,
                    modified_ms: r.get::<_, i64>(5)? as u64,
                    job_id: r.get(6)?,
                })
            })
            .map_err(db)?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(db)
    }
    pub fn forget(&self, source: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM files WHERE source=?1", [source])
            .map_err(db)?;
        self.conn
            .execute("DELETE FROM submitted WHERE source=?1", [source])
            .map_err(db)?;
        self.conn
            .execute("DELETE FROM ignored WHERE source=?1", [source])
            .map_err(db)?;
        Ok(())
    }
}

impl Index {
    pub fn entry(&self, source: &str, relative: &str) -> Result<Entry> {
        self.conn.query_row("SELECT relative,identity,revision,size,mime,modified FROM files WHERE source=?1 AND relative=?2 AND present=1",params![source,relative],|r| {let identity:String=r.get(1)?;Ok(Entry{relative:r.get(0)?,source_id:asset_id(source,&identity),revision:r.get(2)?,size:r.get::<_,i64>(3)? as u64,media_type:r.get(4)?,modified_ms:r.get::<_,i64>(5)? as u64,job_id:None})}).map_err(db)
    }
}

impl Index {
    pub fn baseline(&self, source: &str, receiver: &str) -> Result<()> {
        self.conn.execute("INSERT INTO submitted(source,identity,receiver,revision,job) SELECT source,identity,?2,revision,0 FROM files WHERE source=?1 AND present=1 ON CONFLICT(source,identity,receiver) DO NOTHING",params![source,receiver]).map_err(db)?;
        Ok(())
    }
    pub fn ignore(&self, source: &str, relative: &str, revision: &str) -> Result<()> {
        self.conn.execute("INSERT INTO ignored(source,relative,revision) VALUES(?1,?2,?3) ON CONFLICT(source,relative) DO UPDATE SET revision=excluded.revision,until_ms=0",params![source,relative,revision]).map_err(db)?;
        Ok(())
    }
    pub fn retry_ignored(&self, source: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM ignored WHERE source=?1", [source])
            .map_err(db)?;
        Ok(())
    }
}

impl Index {
    pub fn stable(&self, source: &str, relative: &str, now: u64) -> Result<bool> {
        self.conn
            .query_row(
                "SELECT observed<=?3 FROM files WHERE source=?1 AND relative=?2 AND present=1",
                params![source, relative, now.saturating_sub(10_000) as i64],
                |r| r.get(0),
            )
            .map_err(db)
    }
}

impl Index {
    pub fn job_id(&self, source: &str, relative: &str, receiver: &str) -> Result<Option<i64>> {
        self.conn.query_row("SELECT (SELECT job FROM submitted s WHERE s.source=f.source AND s.identity=f.identity AND (s.receiver=?3 OR s.receiver='@baseline') AND s.revision=f.revision ORDER BY s.job DESC LIMIT 1) FROM files f WHERE source=?1 AND relative=?2 AND present=1",params![source,relative,receiver],|r|r.get(0)).map_err(db)
    }
}

pub fn current_source_id(source: &str, path: &Path, relative: &str) -> Result<String> {
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        let _ = relative;
        fs::metadata(path)?.ino().to_string()
    };
    #[cfg(not(unix))]
    let identity = {
        let _ = path;
        relative.to_owned()
    };
    Ok(asset_id(source, &identity))
}

impl Index {
    pub fn defer(&self, source: &str, relative: &str, revision: &str, until: u64) -> Result<()> {
        self.conn.execute("INSERT INTO ignored(source,relative,revision,until_ms) VALUES(?1,?2,?3,?4) ON CONFLICT(source,relative) DO UPDATE SET revision=excluded.revision,until_ms=excluded.until_ms",params![source,relative,revision,until as i64]).map_err(db)?;
        Ok(())
    }
    pub fn pending(&self, source: &str, receiver: &str) -> Result<i64> {
        self.conn.query_row("SELECT COUNT(*) FROM files f WHERE source=?1 AND present=1 AND NOT EXISTS(SELECT 1 FROM submitted s WHERE s.source=f.source AND s.identity=f.identity AND (s.receiver=?2 OR s.receiver='@baseline') AND s.revision=f.revision) AND NOT EXISTS(SELECT 1 FROM ignored i WHERE i.source=f.source AND i.relative=f.relative AND i.revision=f.revision AND i.until_ms=0)",params![source,receiver],|r|r.get(0)).map_err(db)
    }
}

impl Index {
    pub fn include_existing(&self, source: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM submitted WHERE source=?1 AND job=0", [source])
            .map_err(db)?;
        Ok(())
    }
}
