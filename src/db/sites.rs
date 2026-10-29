//! WordPress sites a Story can be published to (`docs/TRAVEL.md`). A row
//! holds only what isn't secret: the site, its API, the account's user
//! name and what it may do, and the reference its Application Password
//! is kept under in the system's credential store (`src/keychain.rs`).

use rusqlite::{params, OptionalExtension, Row};

use super::Database;
use crate::types::unix_now;

pub(super) const TABLES: &str = "\
CREATE TABLE IF NOT EXISTS publish_sites (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    label TEXT NOT NULL,
    site_url TEXT NOT NULL,
    api_root TEXT NOT NULL,
    user TEXT NOT NULL,
    secret_ref TEXT NOT NULL,
    account TEXT NOT NULL DEFAULT '',
    can_upload INTEGER NOT NULL DEFAULT 0,
    can_edit INTEGER NOT NULL DEFAULT 0,
    can_publish INTEGER NOT NULL DEFAULT 0,
    checked_at INTEGER,
    created_at INTEGER NOT NULL,
    UNIQUE (site_url, user)
);
";

pub(super) fn upgrade(db: &Database) -> Result<(), String> {
    db.conn.execute_batch(TABLES).map_err(|err| err.to_string())
}

/// A connected site, as the library keeps it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Site {
    pub id: i64,
    /// The name the person gave it, such as "Travel blog".
    pub label: String,
    /// The site's address, after redirects within its origin.
    pub site_url: String,
    /// Where its REST API was found.
    pub api_root: String,
    pub user: String,
    /// What the Application Password is kept under in the credential store.
    pub secret_ref: String,
    /// The account's display name, as the site gave it.
    pub account: String,
    pub can_upload: bool,
    pub can_edit: bool,
    pub can_publish: bool,
    /// When the connection was last checked, in seconds since 1970.
    pub checked_at: Option<i64>,
}

const COLS: &str = "id, label, site_url, api_root, user, secret_ref, account, can_upload, can_edit, can_publish, checked_at";

fn read_site(row: &Row) -> rusqlite::Result<Site> {
    Ok(Site {
        id: row.get(0)?,
        label: row.get(1)?,
        site_url: row.get(2)?,
        api_root: row.get(3)?,
        user: row.get(4)?,
        secret_ref: row.get(5)?,
        account: row.get(6)?,
        can_upload: row.get(7)?,
        can_edit: row.get(8)?,
        can_publish: row.get(9)?,
        checked_at: row.get(10)?,
    })
}

impl Database {
    /// The connected sites, by label.
    pub fn sites(&self) -> Vec<Site> {
        let Ok(mut statement) = self.conn.prepare(&format!("SELECT {COLS} FROM publish_sites ORDER BY label COLLATE NOCASE, id")) else {
            return Vec::new();
        };
        statement.query_map([], read_site).map(|rows| rows.filter_map(Result::ok).collect()).unwrap_or_default()
    }

    pub fn site(&self, id: i64) -> Option<Site> {
        self.conn.query_row(&format!("SELECT {COLS} FROM publish_sites WHERE id = ?1"), [id], read_site).optional().ok().flatten()
    }

    /// Adds `site`, or updates the row for the same site and user, and
    /// returns its id. Checking it again is what sets `checked_at`.
    pub fn save_site(&self, site: &Site) -> Result<i64, String> {
        let label = if site.label.trim().is_empty() { site.site_url.as_str() } else { site.label.trim() };
        let existing: Option<i64> = if site.id > 0 {
            Some(site.id)
        } else {
            self.conn
                .query_row("SELECT id FROM publish_sites WHERE site_url = ?1 AND user = ?2", params![site.site_url, site.user], |row| row.get(0))
                .optional()
                .map_err(|err| err.to_string())?
        };
        let values = params![
            label,
            site.site_url,
            site.api_root,
            site.user,
            site.secret_ref,
            site.account,
            site.can_upload,
            site.can_edit,
            site.can_publish,
            site.checked_at,
        ];
        match existing {
            Some(id) => {
                let changed = self
                    .conn
                    .execute(
                        "UPDATE publish_sites SET label = ?1, site_url = ?2, api_root = ?3, user = ?4, secret_ref = ?5, account = ?6,
                         can_upload = ?7, can_edit = ?8, can_publish = ?9, checked_at = ?10 WHERE id = ?11",
                        rusqlite::params_from_iter(values.iter().copied().chain([&id as &dyn rusqlite::ToSql])),
                    )
                    .map_err(|err| err.to_string())?;
                if changed == 0 {
                    return Err("That site is no longer connected".to_string());
                }
                Ok(id)
            }
            None => {
                self.conn
                    .execute(
                        "INSERT INTO publish_sites(label, site_url, api_root, user, secret_ref, account, can_upload, can_edit, can_publish, checked_at, created_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                        rusqlite::params_from_iter(values.iter().copied().chain([&unix_now() as &dyn rusqlite::ToSql])),
                    )
                    .map_err(|err| err.to_string())?;
                Ok(self.conn.last_insert_rowid())
            }
        }
    }

    /// Forgets the site. Its password is removed from the credential store
    /// by the caller, which knows the store.
    pub fn delete_site(&self, id: i64) -> Result<(), String> {
        self.conn.execute("DELETE FROM publish_sites WHERE id = ?1", [id]).map(|_| ()).map_err(|err| err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::library;
    use super::*;

    fn site(url: &str, user: &str) -> Site {
        Site {
            label: "Travel blog".into(),
            site_url: url.into(),
            api_root: format!("{url}/wp-json/"),
            user: user.into(),
            secret_ref: format!("wordpress {user}@{url}"),
            account: "Ana".into(),
            can_upload: true,
            can_edit: true,
            checked_at: Some(100),
            ..Site::default()
        }
    }

    #[test]
    fn a_site_is_kept_without_its_password_and_saved_again_in_place() {
        let (db, _, _) = library("sites");
        let id = db.save_site(&site("https://example.com", "ana")).unwrap();
        let saved = db.site(id).unwrap();
        assert_eq!((saved.label.as_str(), saved.can_upload, saved.can_publish), ("Travel blog", true, false));
        let columns: Vec<String> = db
            .conn
            .prepare("SELECT name FROM pragma_table_info('publish_sites')")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert!(!columns.iter().any(|name| name.contains("password")), "{columns:?}");

        // Connecting the same account again updates its row.
        let again = Site { label: "Blog".into(), can_publish: true, ..site("https://example.com", "ana") };
        assert_eq!(db.save_site(&again).unwrap(), id);
        assert_eq!(db.sites().len(), 1);
        assert!(db.site(id).unwrap().can_publish);

        // Another account on the same site is another row; an empty label is the address.
        let other = db.save_site(&Site { label: " ".into(), ..site("https://example.com", "ben") }).unwrap();
        assert_eq!(db.site(other).unwrap().label, "https://example.com");
        let labels: Vec<String> = db.sites().into_iter().map(|site| site.label).collect();
        assert_eq!(labels, ["Blog", "https://example.com"]);

        db.delete_site(id).unwrap();
        assert_eq!(db.site(id), None);
        assert!(db.save_site(&Site { id, ..site("https://example.com", "ana") }).is_err(), "a removed site isn't saved by its old id");
    }
}
