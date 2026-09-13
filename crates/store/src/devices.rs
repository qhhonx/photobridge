//! Installation-local names and peer display profiles, separate from media IDs.
use photobridge_core::*;
use rusqlite::{params, Connection};
use serde::Serialize;
use std::path::Path;

pub struct DeviceDirectory {
    conn: Connection,
}
#[derive(Clone, Serialize)]
pub struct Peer {
    pub key: String,
    pub profile: DeviceProfile,
    pub last_seen: i64,
    pub ip: Option<String>,
    pub device_type: Option<String>,
    pub enabled: bool,
}
impl DeviceDirectory {
    pub fn open(root: &Path, language: &str) -> Result<Self> {
        std::fs::create_dir_all(root)?;
        let conn = Connection::open(root.join("devices.sqlite3")).map_err(super::db)?;
        conn.busy_timeout(std::time::Duration::from_secs(2))
            .map_err(super::db)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
          CREATE TABLE IF NOT EXISTS local_device(singleton INTEGER PRIMARY KEY CHECK(singleton=1), id TEXT NOT NULL, name TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS peers(peer_key TEXT PRIMARY KEY, id TEXT NOT NULL, name TEXT NOT NULL, last_seen INTEGER NOT NULL);") .map_err(super::db)?;
        conn.execute_batch("CREATE TABLE IF NOT EXISTS peer_details(id TEXT PRIMARY KEY, ip TEXT, device_type TEXT, enabled INTEGER NOT NULL DEFAULT 1);").map_err(super::db)?;
        let exists: bool = conn
            .query_row("SELECT EXISTS(SELECT 1 FROM local_device)", [], |r| {
                r.get(0)
            })
            .map_err(super::db)?;
        if !exists {
            let mut bytes = [0u8; 32];
            getrandom::fill(&mut bytes)
                .map_err(|_| Error::Storage("device identity randomness".into()))?;
            let id: String = bytes.iter().map(|v| format!("{v:02x}")).collect();
            let name = default_name(&bytes, language);
            // Concurrent startup may generate two candidates; exactly one wins.
            conn.execute(
                "INSERT OR IGNORE INTO local_device VALUES(1,?1,?2)",
                params![id, name],
            )
            .map_err(super::db)?;
        }
        Ok(Self { conn })
    }
    pub fn profile(&self) -> Result<DeviceProfile> {
        let profile = self
            .conn
            .query_row(
                "SELECT id,name FROM local_device WHERE singleton=1",
                [],
                |r| {
                    Ok(DeviceProfile {
                        id: r.get(0)?,
                        name: r.get(1)?,
                    })
                },
            )
            .map_err(super::db)?;
        profile.validate()?;
        Ok(profile)
    }
    pub fn rename(&self, name: &str) -> Result<DeviceProfile> {
        let mut profile = self.profile()?;
        profile.name = name.trim().into();
        profile.validate()?;
        self.conn
            .execute(
                "UPDATE local_device SET name=?1 WHERE singleton=1",
                [&profile.name],
            )
            .map_err(super::db)?;
        Ok(profile)
    }
    pub fn remember(&self, key: &str, profile: &DeviceProfile) -> Result<()> {
        profile.validate()?;
        if !valid_digest(key) {
            return Err(Error::Invalid("peer key".into()));
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .min(i64::MAX as u64) as i64;
        // Bounded display cache; eviction never touches pairing credentials.
        self.conn
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(super::db)?;
        let saved = (|| {
            self.conn.execute("INSERT INTO peers VALUES(?1,?2,?3,?4) ON CONFLICT(peer_key) DO UPDATE SET id=excluded.id,name=excluded.name,last_seen=excluded.last_seen",params![key,profile.id,profile.name,now]).map_err(super::db)?;
            self.conn.execute("DELETE FROM peers WHERE peer_key NOT IN (SELECT peer_key FROM peers ORDER BY last_seen DESC, rowid DESC LIMIT 256)",[]).map_err(super::db)?;
            Ok(())
        })();
        if saved.is_ok() {
            self.conn.execute_batch("COMMIT").map_err(super::db)?;
        } else {
            let _ = self.conn.execute_batch("ROLLBACK");
        }
        saved
    }
    /// This is a reversible reception policy, not credential revocation.
    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        if !valid_digest(id) {
            return Err(Error::Invalid("sender id".into()));
        }
        let known: bool = self
            .conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM peers WHERE id=?1)",
                [id],
                |r| r.get(0),
            )
            .map_err(super::db)?;
        if !known {
            return Err(Error::NotFound);
        }
        self.conn.execute("INSERT INTO peer_details(id,enabled) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET enabled=excluded.enabled", params![id,enabled]).map_err(super::db)?;
        Ok(())
    }
    pub fn enabled(&self, id: &str) -> Result<bool> {
        if !valid_digest(id) {
            return Err(Error::Invalid("sender id".into()));
        }
        self.conn
            .query_row(
                "SELECT COALESCE((SELECT enabled FROM peer_details WHERE id=?1),1)",
                [id],
                |r| r.get(0),
            )
            .map_err(super::db)
    }
    pub fn observe(
        &self,
        id: &str,
        ip: Option<std::net::IpAddr>,
        device_type: Option<&str>,
    ) -> Result<()> {
        if !valid_digest(id)
            || device_type.is_some_and(|v| {
                !["iPhone", "iPad", "Mac", "Android", "Windows", "Linux"].contains(&v)
            })
        {
            return Err(Error::Invalid("sender metadata".into()));
        }
        self.conn.execute("INSERT INTO peer_details(id,ip,device_type) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET ip=COALESCE(excluded.ip,ip),device_type=COALESCE(excluded.device_type,device_type)",params![id,ip.map(|v|v.to_string()),device_type]).map_err(super::db)?;
        self.conn
            .execute("UPDATE peers SET last_seen=unixepoch() WHERE id=?1", [id])
            .map_err(super::db)?;
        Ok(())
    }
    pub fn peers(&self) -> Result<Vec<Peer>> {
        let mut query = self.conn.prepare("SELECT p.peer_key,p.id,p.name,p.last_seen,d.ip,d.device_type,COALESCE(d.enabled,1) FROM peers p LEFT JOIN peer_details d ON p.id=d.id ORDER BY p.last_seen DESC,p.rowid DESC LIMIT 256").map_err(super::db)?;
        let rows = query
            .query_map([], |r| {
                Ok(Peer {
                    key: r.get(0)?,
                    profile: DeviceProfile {
                        id: r.get(1)?,
                        name: r.get(2)?,
                    },
                    last_seen: r.get(3)?,
                    ip: r.get(4)?,
                    device_type: r.get(5)?,
                    enabled: r.get(6)?,
                })
            })
            .map_err(super::db)?
            .collect::<std::result::Result<_, _>>()
            .map_err(super::db)?;
        Ok(rows)
    }
}

fn default_name(bytes: &[u8; 32], language: &str) -> String {
    let en_a = [
        "Amber", "Moonlit", "Silver", "Sunny", "Misty", "Coral", "Velvet", "Quiet", "Golden",
        "Jade", "Indigo", "Gentle", "Starry", "Snowy", "Emerald", "Dawn",
    ];
    let en_b = [
        "Otter", "Cedar", "Panda", "Robin", "Fox", "Maple", "Heron", "Willow", "Lynx", "Comet",
        "Badger", "Orchid", "Finch", "Juniper", "Dolphin", "Meadow",
    ];
    let zh_a = [
        "琥珀", "月光", "银色", "晴空", "薄雾", "珊瑚", "丝绒", "静谧", "金色", "翡翠", "靛蓝",
        "轻风", "星夜", "白雪", "青绿", "晨光",
    ];
    let zh_b = [
        "水獭",
        "雪松",
        "熊猫",
        "知更鸟",
        "狐狸",
        "枫树",
        "白鹭",
        "柳树",
        "山猫",
        "彗星",
        "小獾",
        "兰花",
        "雀鸟",
        "杜松",
        "海豚",
        "草原",
    ];
    let a = usize::from(bytes[0] & 15);
    let b = usize::from(bytes[1] & 15);
    if language.starts_with("zh") {
        format!("{}{} · {:02X}{:02X}", zh_a[a], zh_b[b], bytes[2], bytes[3])
    } else {
        format!("{} {} · {:02X}{:02X}", en_a[a], en_b[b], bytes[2], bytes[3])
    }
}
