//! Local archive access is independent of the network listener. Both paths use
//! the same Receiver integrity checks and exclusive writer lock.
use super::*;

pub(super) fn with_store<T>(
    root: Option<&Path>,
    operation: impl FnOnce(&mut Receiver, &maintenance::Maintenance) -> Result<T>,
) -> Result<T> {
    // Serialize against listener start/stop and reuse its writer when present.
    let hosts = HOSTS.lock().map_err(lock)?;
    if let Some(host) = hosts
        .receiver
        .as_ref()
        .filter(|h| root.is_none_or(|r| r == h.root))
    {
        let mut store = host.receiver.lock().map_err(lock)?;
        let maintenance = host.maintenance.lock().map_err(lock)?;
        return operation(&mut store, &maintenance);
    }
    let root = root.ok_or(Error::NotFound)?;
    // An empty, never-started receiver has nothing to archive. Do not create a
    // new catalog (or identity) merely because an archive command was requested.
    if !root.join("store/receiver.sqlite3").is_file() {
        return Err(Error::NotFound);
    }
    let maintenance = maintenance::Maintenance::open(root)?;
    let mut store = Receiver::open(
        root.join("store"),
        maintenance.settings.receiver_budget_bytes,
    )?;
    store.configure_storage(
        maintenance.settings.receiver_budget_bytes,
        maintenance.settings.min_free_bytes,
    )?;
    operation(&mut store, &maintenance)
}

pub(super) fn with_maintenance<T>(
    root: Option<&Path>,
    operation: impl FnOnce(&maintenance::Maintenance) -> Result<T>,
) -> Result<T> {
    let hosts = HOSTS.lock().map_err(lock)?;
    if let Some(host) = hosts
        .receiver
        .as_ref()
        .filter(|h| root.is_none_or(|r| r == h.root))
    {
        let maintenance = host.maintenance.lock().map_err(lock)?;
        return operation(&maintenance);
    }
    let root = root.ok_or(Error::NotFound)?;
    if !root.join("store/receiver.sqlite3").is_file() {
        return Err(Error::NotFound);
    }
    operation(&maintenance::Maintenance::open(root)?)
}
