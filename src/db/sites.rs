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
-- Photos uploaded to a site, by the SHA-256 of the bytes sent, so a retry
-- or another Story reuses a confirmed upload. Kept when the site or Story
-- goes: the uploads stay on the site.
CREATE TABLE IF NOT EXISTS publish_media (
    site_id INTEGER NOT NULL,
    hash TEXT NOT NULL,
    size TEXT NOT NULL,
    media_id INTEGER NOT NULL,
    url TEXT NOT NULL,
    uploaded_at INTEGER NOT NULL,
    PRIMARY KEY (site_id, hash)
);
-- A Story's post on a site. `output_key` and `edition` leave room for
-- more posts per Story; there is one main post, edition 1, for now.
CREATE TABLE IF NOT EXISTS publications (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    story_id INTEGER NOT NULL,
    site_id INTEGER NOT NULL,
    output_key TEXT NOT NULL DEFAULT 'main',
    edition INTEGER NOT NULL DEFAULT 1,
    post_id INTEGER NOT NULL,
    remote_status TEXT NOT NULL DEFAULT '',
    size TEXT NOT NULL,
    revision INTEGER NOT NULL,
    synced_at INTEGER NOT NULL,
    UNIQUE (story_id, site_id, output_key, edition),
    UNIQUE (site_id, post_id)
);
CREATE INDEX IF NOT EXISTS idx_publications_story ON publications(story_id);
-- The site's categories and tags chosen for a Story's post, with their names
-- as the site gave them. None chosen leaves the post's own as they are.
CREATE TABLE IF NOT EXISTS publication_terms (
    story_id INTEGER NOT NULL,
    site_id INTEGER NOT NULL,
    taxonomy TEXT NOT NULL,
    term_id INTEGER NOT NULL,
    name TEXT NOT NULL,
    PRIMARY KEY (story_id, site_id, taxonomy, term_id)
);
";

pub(super) fn upgrade(db: &Database) -> Result<(), String> {
    db.conn.execute_batch(TABLES).map_err(|err| err.to_string())?;
    // What Picasso wrote in the post as the site kept it, and whether it was
    // found edited there since.
    db.ensure_column("publications", "fingerprint", "TEXT NOT NULL DEFAULT ''")?;
    db.ensure_column("publications", "changed_on_site", "INTEGER NOT NULL DEFAULT 0")
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

/// How `publication_terms` names the two taxonomies, as WordPress does.
const CATEGORY: &str = "category";
const TAG: &str = "post_tag";

/// The categories and tags chosen for a post, each as (term id, name).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PostTerms {
    pub categories: Vec<(i64, String)>,
    pub tags: Vec<(i64, String)>,
}

/// A Story's post on a site, as last saved there.
#[derive(Clone, Debug, PartialEq)]
pub struct Publication {
    pub story_id: i64,
    pub site_id: i64,
    pub post_id: i64,
    /// The status the site last reported, such as "draft".
    pub remote_status: String,
    /// The upload size's code (`UploadSize::code`).
    pub size: String,
    /// The Story revision it was made from.
    pub revision: i64,
    pub synced_at: i64,
    /// What Picasso wrote in the post as the site kept it after the last
    /// save (`wordpress::fingerprint`); empty for one saved before it was kept.
    pub fingerprint: String,
    /// The post was found edited on the site since that save.
    pub changed_on_site: bool,
}

impl Database {
    /// The confirmed uploads of site `site_id`, by the hash of their bytes:
    /// (media id, address).
    pub fn uploads(&self, site_id: i64) -> std::collections::HashMap<String, (i64, String)> {
        let Ok(mut statement) = self.conn.prepare("SELECT hash, media_id, url FROM publish_media WHERE site_id = ?1") else {
            return Default::default();
        };
        statement
            .query_map([site_id], |row| Ok((row.get::<_, String>(0)?, (row.get(1)?, row.get(2)?))))
            .map(|rows| rows.filter_map(Result::ok).collect())
            .unwrap_or_default()
    }

    /// Records an upload the site confirmed, the moment it did.
    pub fn record_upload(&self, site_id: i64, hash: &str, size: &str, media_id: i64, url: &str) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO publish_media(site_id, hash, size, media_id, url, uploaded_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![site_id, hash, size, media_id, url, unix_now()],
            )
            .map(|_| ())
            .map_err(|err| err.to_string())
    }

    /// Forgets an upload the site no longer has.
    pub fn forget_upload(&self, site_id: i64, hash: &str) -> Result<(), String> {
        self.conn.execute("DELETE FROM publish_media WHERE site_id = ?1 AND hash = ?2", params![site_id, hash]).map(|_| ()).map_err(|err| err.to_string())
    }

    /// Story `story_id`'s main post on site `site_id`, if one was made.
    pub fn publication(&self, story_id: i64, site_id: i64) -> Option<Publication> {
        self.conn
            .query_row(
                "SELECT story_id, site_id, post_id, remote_status, size, revision, synced_at, fingerprint, changed_on_site FROM publications
                 WHERE story_id = ?1 AND site_id = ?2 AND output_key = 'main' AND edition = 1",
                params![story_id, site_id],
                |row| {
                    Ok(Publication {
                        story_id: row.get(0)?,
                        site_id: row.get(1)?,
                        post_id: row.get(2)?,
                        remote_status: row.get(3)?,
                        size: row.get(4)?,
                        revision: row.get(5)?,
                        synced_at: row.get(6)?,
                        fingerprint: row.get(7)?,
                        changed_on_site: row.get(8)?,
                    })
                },
            )
            .optional()
            .ok()
            .flatten()
    }

    /// Forgets Story `story_id`'s post on site `site_id`, once the site no longer has it.
    pub fn forget_publication(&self, story_id: i64, site_id: i64) -> Result<(), String> {
        self.conn
            .execute("DELETE FROM publications WHERE story_id = ?1 AND site_id = ?2 AND output_key = 'main' AND edition = 1", params![story_id, site_id])
            .map(|_| ())
            .map_err(|err| err.to_string())
    }

    /// The categories and tags chosen for the Story's post on the site.
    pub fn post_terms(&self, story_id: i64, site_id: i64) -> PostTerms {
        let mut terms = PostTerms::default();
        let Ok(mut statement) = self.conn.prepare(
            "SELECT taxonomy, term_id, name FROM publication_terms WHERE story_id = ?1 AND site_id = ?2 ORDER BY name COLLATE NOCASE, term_id",
        ) else {
            return terms;
        };
        let rows = statement.query_map(params![story_id, site_id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?)));
        for (taxonomy, id, name) in rows.into_iter().flatten().flatten() {
            match taxonomy.as_str() {
                CATEGORY => terms.categories.push((id, name)),
                TAG => terms.tags.push((id, name)),
                _ => {}
            }
        }
        terms
    }

    /// Replaces the categories and tags chosen for the Story's post on the site.
    pub fn save_post_terms(&self, story_id: i64, site_id: i64, terms: &PostTerms) -> Result<(), String> {
        let tx = self.conn.unchecked_transaction().map_err(|err| err.to_string())?;
        tx.execute("DELETE FROM publication_terms WHERE story_id = ?1 AND site_id = ?2", params![story_id, site_id]).map_err(|err| err.to_string())?;
        for (taxonomy, list) in [(CATEGORY, &terms.categories), (TAG, &terms.tags)] {
            for (id, name) in list {
                tx.execute(
                    "INSERT OR REPLACE INTO publication_terms(story_id, site_id, taxonomy, term_id, name) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![story_id, site_id, taxonomy, id, name],
                )
                .map_err(|err| err.to_string())?;
            }
        }
        tx.commit().map_err(|err| err.to_string())
    }

    /// Records the main post of a Story on a site, the moment the site confirmed it.
    pub fn save_publication(&self, publication: &Publication) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO publications(story_id, site_id, post_id, remote_status, size, revision, synced_at, fingerprint, changed_on_site)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT (story_id, site_id, output_key, edition) DO UPDATE SET
                 post_id = excluded.post_id, remote_status = excluded.remote_status, size = excluded.size,
                 revision = excluded.revision, synced_at = excluded.synced_at,
                 fingerprint = excluded.fingerprint, changed_on_site = excluded.changed_on_site",
                params![
                    publication.story_id,
                    publication.site_id,
                    publication.post_id,
                    publication.remote_status,
                    publication.size,
                    publication.revision,
                    publication.synced_at,
                    publication.fingerprint,
                    publication.changed_on_site
                ],
            )
            .map(|_| ())
            .map_err(|err| err.to_string())
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

    #[test]
    fn uploads_and_posts_are_remembered_per_site() {
        let (db, _, _) = library("publications");
        let site_id = db.save_site(&site("https://example.com", "ana")).unwrap();
        db.record_upload(site_id, "abc", "low", 41, "https://example.com/a.jpg").unwrap();
        assert_eq!(db.uploads(site_id).get("abc"), Some(&(41, "https://example.com/a.jpg".to_string())));
        assert!(db.uploads(site_id + 1).is_empty(), "another site has its own uploads");
        db.forget_upload(site_id, "abc").unwrap();
        assert!(db.uploads(site_id).is_empty());

        let mut publication = Publication { story_id: 3, site_id, post_id: 7, remote_status: "draft".into(), size: "standard".into(), revision: 4, synced_at: 100, fingerprint: "ab".into(), changed_on_site: false };
        db.save_publication(&publication).unwrap();
        publication.revision = 5;
        publication.changed_on_site = true;
        db.save_publication(&publication).unwrap();
        assert_eq!(db.publication(3, site_id), Some(publication.clone()));
        assert_eq!(db.publication(3, site_id + 1), None);
        // Another Story can't claim the same post.
        assert!(db.save_publication(&Publication { story_id: 4, ..publication }).is_err());

        // Its categories and tags, kept per Story and site and replaced as a whole.
        assert_eq!(db.post_terms(3, site_id), PostTerms::default());
        let terms = PostTerms { categories: vec![(5, "Spain".into()), (2, "Europe".into())], tags: vec![(9, "Alhambra".into())] };
        db.save_post_terms(3, site_id, &terms).unwrap();
        let read = db.post_terms(3, site_id);
        assert_eq!(read.categories, [(2, "Europe".to_string()), (5, "Spain".to_string())], "by name");
        assert_eq!(read.tags, terms.tags);
        assert_eq!(db.post_terms(4, site_id), PostTerms::default());
        db.save_post_terms(3, site_id, &PostTerms { categories: vec![(2, "Europe".into())], tags: Vec::new() }).unwrap();
        assert_eq!(db.post_terms(3, site_id).tags, []);

        // Disconnecting the site keeps what is known to be on it.
        db.delete_site(site_id).unwrap();
        assert!(db.publication(3, site_id).is_some());
    }
}
