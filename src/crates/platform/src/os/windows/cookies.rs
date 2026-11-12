use super::dpapi::{decrypt_blob, read_os_crypt_key};
use super::{app_support, claude_desktop_bases, home_dir};
use agent_hud_agents::Answer;
use rusqlite::{Connection, OpenFlags};
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const CHROMIUM_EPOCH_OFFSET_US: i64 = 11_644_473_600 * 1_000_000;
static TEMP_COOKIE_SEQ: AtomicU64 = AtomicU64::new(0);

struct TempDbCopy {
    dir: PathBuf,
    db: PathBuf,
}

impl Drop for TempDbCopy {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn copy_sqlite_files(src: &Path) -> Option<TempDbCopy> {
    let seq = TEMP_COOKIE_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("agent-hud-cookies-{}-{seq}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let db = dir.join("Cookies");
    if std::fs::copy(src, &db).is_err() {
        let _ = std::fs::remove_dir_all(&dir);
        return None;
    }
    for suffix in ["-wal", "-shm"] {
        let side = PathBuf::from(format!("{}{suffix}", src.display()));
        if side.is_file() {
            let _ = std::fs::copy(&side, dir.join(format!("Cookies{suffix}")));
        }
    }
    Some(TempDbCopy { dir, db })
}

fn open_cookie_db(path: &Path) -> Result<(Connection, Option<TempDbCopy>), ()> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let has_wal = PathBuf::from(format!("{}-wal", path.display())).is_file();
    if let Some(conn) = (!has_wal)
        .then(|| Connection::open_with_flags(path, flags).ok())
        .flatten()
    {
        let _ = conn.busy_timeout(Duration::from_millis(200));
        return Ok((conn, None));
    }
    let copy = copy_sqlite_files(path).ok_or(())?;
    let conn = Connection::open_with_flags(&copy.db, flags).map_err(|_| ())?;
    Ok((conn, Some(copy)))
}

fn now_chromium_utc_us() -> i64 {
    let unix_us = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as i64;
    unix_us.saturating_add(CHROMIUM_EPOCH_OFFSET_US)
}

fn candidate_bases(app_data: &Path, app: &str) -> Vec<PathBuf> {
    let mut bases = vec![app_data.join(app)];
    if app != "Claude" {
        return bases;
    }
    let Ok(home) = home_dir() else {
        return bases;
    };
    let env = super::current_env();
    if app_data != app_support(&home, &env) {
        return bases;
    }
    for extra in claude_desktop_bases(&home, &env) {
        if !bases.contains(&extra) {
            bases.push(extra);
        }
    }
    bases
}

fn cookie_db_in_base(base: &Path) -> Option<PathBuf> {
    [base.join("Network/Cookies"), base.join("Cookies")]
        .into_iter()
        .find(|path| path.is_file())
}

fn host_matches(host_key: &str, domain: &str, dotted: &str) -> Option<u8> {
    if host_key == domain {
        Some(0)
    } else if host_key == dotted {
        Some(1)
    } else if host_key.ends_with(dotted) {
        Some(2)
    } else {
        None
    }
}

fn read_cookie_from_base(base: &Path, site: &str, name: &str) -> Answer<String> {
    let Some(db_path) = cookie_db_in_base(base) else {
        return Answer::Missing;
    };
    let Ok((conn, _temp)) = open_cookie_db(&db_path) else {
        return Answer::Failed;
    };
    let db_version: i64 = conn
        .query_row(
            "SELECT CAST(value AS INTEGER) FROM meta WHERE key = 'version'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let strip_host_hash = db_version >= 24;
    let Ok(mut stmt) = conn.prepare(
        "SELECT value, encrypted_value, host_key, expires_utc \
         FROM cookies WHERE name = ?1 ORDER BY expires_utc DESC",
    ) else {
        return Answer::Failed;
    };
    let domain = site.trim_start_matches('.');
    let dotted = format!(".{domain}");
    let now_us = now_chromium_utc_us();
    let Ok(rows) = stmt.query_map([name], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Vec<u8>>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
        ))
    }) else {
        return Answer::Failed;
    };
    let mut matching = Vec::new();
    for row in rows {
        let Ok((value, encrypted, host_key, expires_utc)) = row else {
            return Answer::Failed;
        };
        if expires_utc > 0 && expires_utc < now_us {
            continue;
        }
        if let Some(rank) = host_matches(&host_key, domain, &dotted) {
            matching.push((rank, expires_utc, value, encrypted, host_key));
        }
    }
    matching.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.cmp(&a.1)));

    let mut cached_key: Option<Answer<Vec<u8>>> = None;
    let mut outcome = Answer::Missing;
    for (_, _, value, encrypted, host_key) in matching {
        if !value.is_empty() {
            return Answer::Found(value);
        }
        if encrypted.is_empty() {
            continue;
        }
        let key_ref =
            cached_key.get_or_insert_with(|| read_os_crypt_key(&base.join("Local State")));
        match decrypt_blob(&encrypted, Some(key_ref), strip_host_hash, Some(&host_key)) {
            Answer::Found(found) => return Answer::Found(found),
            other => outcome = other,
        }
    }
    outcome
}

pub(crate) fn app_cookie(app_data: &Path, app: &str, site: &str, name: &str) -> Answer<String> {
    let mut outcome = Answer::Missing;
    for base in candidate_bases(app_data, app) {
        match read_cookie_from_base(&base, site, name) {
            Answer::Found(found) => return Answer::Found(found),
            Answer::Missing => {}
            other => outcome = other,
        }
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::dpapi_tests::{aes_256_gcm_encrypt, protect_dpapi};
    use super::*;
    use base64::{engine::general_purpose::STANDARD, Engine};
    use sha2::{Digest, Sha256};
    use tempfile::tempdir;

    #[test]
    fn reads_and_decrypts_v24_electron_cookies_and_skips_expired_ones() {
        let tmp = tempdir().unwrap();
        let app_dir = tmp.path().join("Claude");
        let network_dir = app_dir.join("Network");
        std::fs::create_dir_all(&network_dir).unwrap();

        let key = [0x21u8; 32];
        let mut raw_key = b"DPAPI".to_vec();
        raw_key.extend_from_slice(&protect_dpapi(&key));
        let state = serde_json::json!({
            "os_crypt": { "encrypted_key": STANDARD.encode(&raw_key) }
        });
        std::fs::write(app_dir.join("Local State"), state.to_string()).unwrap();

        let conn = Connection::open(network_dir.join("Cookies")).unwrap();
        conn.execute_batch(
            "CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT);
             INSERT INTO meta(key, value) VALUES ('version', '24');
             CREATE TABLE cookies(host_key TEXT, name TEXT, value TEXT, encrypted_value BLOB, expires_utc INTEGER);",
        )
        .unwrap();

        let mut payload = Sha256::digest(b".claude.ai").to_vec();
        payload.extend_from_slice(b"sk-ant-sid-cookie");
        let sealed = aes_256_gcm_encrypt(&key, b"nonce-123456", &payload);
        let future_us = now_chromium_utc_us() + 60_000_000;

        conn.execute(
            "INSERT INTO cookies VALUES (?1, ?2, '', ?3, ?4)",
            rusqlite::params![".claude.ai", "sessionKey", sealed, future_us],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO cookies VALUES (?1, ?2, 'stale', X'', ?3)",
            rusqlite::params![".claude.ai", "expiredKey", 100],
        )
        .unwrap();
        drop(conn);

        assert_eq!(
            app_cookie(tmp.path(), "Claude", "claude.ai", "sessionKey"),
            Answer::Found("sk-ant-sid-cookie".into())
        );
        assert_eq!(
            app_cookie(tmp.path(), "Claude", "claude.ai", "expiredKey"),
            Answer::Missing
        );
        assert_eq!(
            app_cookie(tmp.path(), "Claude", "claude.ai", "absent"),
            Answer::Missing
        );
    }
}
