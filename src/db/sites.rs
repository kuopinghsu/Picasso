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
-- The site's media a Story's post uses, as of its last confirmed save, so
-- what a post depends on is known; nothing is deleted on the site.
CREATE TABLE IF NOT EXISTS publication_media (
    story_id INTEGER NOT NULL,
    site_id INTEGER NOT NULL,
    media_id INTEGER NOT NULL,
    PRIMARY KEY (story_id, site_id, media_id)
);
-- Each publishing job from when it starts, for looking back: nothing secret,
-- only what happened. The newest PUBLISH_LOG_KEPT are kept.
CREATE TABLE IF NOT EXISTS publish_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    story_id INTEGER NOT NULL,
    site_id INTEGER NOT NULL,
    started_at INTEGER NOT NULL,
    ended_at INTEGER NOT NULL,
    outcome TEXT NOT NULL,
    post_id INTEGER,
    uploaded INTEGER NOT NULL,
    reused INTEGER NOT NULL,
    size TEXT NOT NULL,
    revision INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_publish_log_story ON publish_log(story_id, site_id);
-- A draft about to be made for a Story on a site whose making the site
-- hasn't confirmed: one made since `since` is looked for before another is.
CREATE TABLE IF NOT EXISTS publish_pending (
    story_id INTEGER NOT NULL,
    site_id INTEGER NOT NULL,
    since INTEGER NOT NULL,
    PRIMARY KEY (story_id, site_id)
);
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
-- What a Story's posts on a site show of where it went, as chosen when
-- publishing: its Places on each post's map when `places` is 1, to about a
-- kilometre when `rough` is 1, but not the Places whose ids `left_out`
-- lists by commas; and whether the posts link to the Trip's Journey.
CREATE TABLE IF NOT EXISTS publication_places (
    story_id INTEGER NOT NULL,
    site_id INTEGER NOT NULL,
    places INTEGER NOT NULL DEFAULT 0,
    rough INTEGER NOT NULL DEFAULT 0,
    left_out TEXT NOT NULL DEFAULT '',
    journey INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (story_id, site_id)
);
-- A Trip's Journey on a site: a post of the VoyageLight theme's `vl_trip`
-- type that lists the Trip's Stories and their Places by Day.
CREATE TABLE IF NOT EXISTS publish_journeys (
    trip_id INTEGER NOT NULL,
    site_id INTEGER NOT NULL,
    post_id INTEGER NOT NULL,
    PRIMARY KEY (trip_id, site_id)
);
";

pub(super) fn upgrade(db: &Database) -> Result<(), String> {
    db.conn.execute_batch(TABLES).map_err(|err| err.to_string())?;
    // What Picasso wrote in the post as the site kept it, and whether it was
    // found edited there since.
    db.ensure_column("publications", "fingerprint", "TEXT NOT NULL DEFAULT ''")?;
    db.ensure_column("publications", "changed_on_site", "INTEGER NOT NULL DEFAULT 0")?;
    // What the site made of an upload: its size, type and the smaller sizes it made.
    db.ensure_column("publish_media", "details", "TEXT NOT NULL DEFAULT ''")
}

/// Publishing jobs kept in `publish_log`; older ones are dropped.
const PUBLISH_LOG_KEPT: i64 = 1000;

/// One publishing job, as it ended or as far as it got.
#[derive(Clone, Debug, PartialEq)]
pub struct PublishJob {
    pub story_id: i64,
    pub site_id: i64,
    pub started_at: i64,
    /// When it ended, or when its last upload landed while it ran.
    pub ended_at: i64,
    /// How it ended, such as "saved", "published" or "cancelled"
    /// (`publish::outcome_code`), or `publish::RUNNING` until it does.
    pub outcome: String,
    pub post_id: Option<i64>,
    pub uploaded: usize,
    pub reused: usize,
    /// The upload size's code.
    pub size: String,
    pub revision: i64,
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

/// What a Story's posts on a site show of where it went; see `publication_places`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlaceChoice {
    /// The Story's Places on each post's map.
    pub places: bool,
    /// Kept to about a kilometre.
    pub rough: bool,
    /// Places not shown, by id.
    pub left_out: Vec<i64>,
    /// The posts link to the Trip's Journey.
    pub journey: bool,
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
    pub fn record_upload(&self, site_id: i64, hash: &str, size: &str, media_id: i64, url: &str, details: &str) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO publish_media(site_id, hash, size, media_id, url, uploaded_at, details) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![site_id, hash, size, media_id, url, unix_now(), details],
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

    /// The site's media the Story's post used at its last confirmed save.
    pub fn publication_media(&self, story_id: i64, site_id: i64) -> Vec<i64> {
        self.map_rows("SELECT media_id FROM publication_media WHERE story_id = ?1 AND site_id = ?2 ORDER BY media_id", params![story_id, site_id], |row| row.get(0))
    }

    /// Replaces the media the Story's post uses, after a confirmed save.
    pub fn save_publication_media(&self, story_id: i64, site_id: i64, media: &[i64]) -> Result<(), String> {
        let tx = self.conn.unchecked_transaction().map_err(|err| err.to_string())?;
        tx.execute("DELETE FROM publication_media WHERE story_id = ?1 AND site_id = ?2", params![story_id, site_id]).map_err(|err| err.to_string())?;
        for id in media {
            tx.execute("INSERT OR IGNORE INTO publication_media(story_id, site_id, media_id) VALUES (?1, ?2, ?3)", params![story_id, site_id, id]).map_err(|err| err.to_string())?;
        }
        tx.commit().map_err(|err| err.to_string())
    }

    /// Keeps a publishing job, dropping the oldest beyond [`PUBLISH_LOG_KEPT`].
    /// A job is kept as it starts, as `publish::RUNNING`, so one cut short by
    /// a crash or a power cut stays in the log. Returns its row.
    pub fn record_publish_job(&self, job: &PublishJob) -> Result<i64, String> {
        let tx = self.conn.unchecked_transaction().map_err(|err| err.to_string())?;
        tx.execute(
            "INSERT INTO publish_log(story_id, site_id, started_at, ended_at, outcome, post_id, uploaded, reused, size, revision)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![job.story_id, job.site_id, job.started_at, job.ended_at, job.outcome, job.post_id, job.uploaded as i64, job.reused as i64, job.size, job.revision],
        )
        .map_err(|err| err.to_string())?;
        let id = tx.last_insert_rowid();
        tx.execute("DELETE FROM publish_log WHERE id <= (SELECT MAX(id) FROM publish_log) - ?1", [PUBLISH_LOG_KEPT]).map_err(|err| err.to_string())?;
        tx.commit().map_err(|err| err.to_string())?;
        Ok(id)
    }

    /// Counts an upload of job `id` the moment the site confirms it.
    pub fn count_publish_upload(&self, id: i64) -> Result<(), String> {
        self.conn
            .execute("UPDATE publish_log SET uploaded = uploaded + 1, ended_at = ?2 WHERE id = ?1", params![id, unix_now()])
            .map(|_| ())
            .map_err(|err| err.to_string())
    }

    /// Keeps how job `id` ended.
    pub fn finish_publish_job(&self, id: i64, job: &PublishJob) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE publish_log SET ended_at = ?2, outcome = ?3, post_id = ?4, uploaded = ?5, reused = ?6 WHERE id = ?1",
                params![id, job.ended_at, job.outcome, job.post_id, job.uploaded as i64, job.reused as i64],
            )
            .map(|_| ())
            .map_err(|err| err.to_string())
    }

    /// The Story's publishing jobs on the site, newest first.
    pub fn publish_jobs(&self, story_id: i64, site_id: i64) -> Vec<PublishJob> {
        let Ok(mut statement) = self.conn.prepare(
            "SELECT story_id, site_id, started_at, ended_at, outcome, post_id, uploaded, reused, size, revision
             FROM publish_log WHERE story_id = ?1 AND site_id = ?2 ORDER BY id DESC",
        ) else {
            return Vec::new();
        };
        let rows = statement.query_map(params![story_id, site_id], |row| {
            Ok(PublishJob {
                story_id: row.get(0)?,
                site_id: row.get(1)?,
                started_at: row.get(2)?,
                ended_at: row.get(3)?,
                outcome: row.get(4)?,
                post_id: row.get(5)?,
                uploaded: row.get::<_, i64>(6)? as usize,
                reused: row.get::<_, i64>(7)? as usize,
                size: row.get(8)?,
                revision: row.get(9)?,
            })
        });
        rows.into_iter().flatten().flatten().collect()
    }

    /// When a draft was about to be made for the Story on the site without
    /// the site confirming it.
    pub fn pending_draft(&self, story_id: i64, site_id: i64) -> Option<i64> {
        self.conn
            .query_row("SELECT since FROM publish_pending WHERE story_id = ?1 AND site_id = ?2", params![story_id, site_id], |row| row.get(0))
            .optional()
            .ok()
            .flatten()
    }

    /// Keeps that a draft is about to be made, before it is, the earliest
    /// moment kept when one is already.
    pub fn set_pending_draft(&self, story_id: i64, site_id: i64, since: i64) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO publish_pending(story_id, site_id, since) VALUES (?1, ?2, ?3)
                 ON CONFLICT (story_id, site_id) DO UPDATE SET since = MIN(since, excluded.since)",
                params![story_id, site_id, since],
            )
            .map(|_| ())
            .map_err(|err| err.to_string())
    }

    /// Forgets it once the site confirmed the draft.
    pub fn clear_pending_draft(&self, story_id: i64, site_id: i64) -> Result<(), String> {
        self.conn.execute("DELETE FROM publish_pending WHERE story_id = ?1 AND site_id = ?2", params![story_id, site_id]).map(|_| ()).map_err(|err| err.to_string())
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

    /// What the Story's posts on the site show of where it went.
    pub fn place_choice(&self, story_id: i64, site_id: i64) -> PlaceChoice {
        self.conn
            .query_row(
                "SELECT places, rough, left_out, journey FROM publication_places WHERE story_id = ?1 AND site_id = ?2",
                params![story_id, site_id],
                |row| {
                    let left_out: String = row.get(2)?;
                    Ok(PlaceChoice {
                        places: row.get(0)?,
                        rough: row.get(1)?,
                        left_out: left_out.split(',').filter_map(|id| id.trim().parse().ok()).collect(),
                        journey: row.get(3)?,
                    })
                },
            )
            .optional()
            .ok()
            .flatten()
            .unwrap_or_default()
    }

    pub fn save_place_choice(&self, story_id: i64, site_id: i64, choice: &PlaceChoice) -> Result<(), String> {
        let left_out = choice.left_out.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
        self.conn
            .execute(
                "INSERT OR REPLACE INTO publication_places(story_id, site_id, places, rough, left_out, journey) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![story_id, site_id, choice.places, choice.rough, left_out, choice.journey],
            )
            .map(|_| ())
            .map_err(|err| err.to_string())
    }

    /// The post of the Trip's Journey on the site, once made.
    pub fn journey_post(&self, trip_id: i64, site_id: i64) -> Option<i64> {
        self.conn
            .query_row("SELECT post_id FROM publish_journeys WHERE trip_id = ?1 AND site_id = ?2", params![trip_id, site_id], |row| row.get(0))
            .optional()
            .ok()
            .flatten()
    }

    pub fn set_journey_post(&self, trip_id: i64, site_id: i64, post_id: i64) -> Result<(), String> {
        self.conn
            .execute("INSERT OR REPLACE INTO publish_journeys(trip_id, site_id, post_id) VALUES (?1, ?2, ?3)", params![trip_id, site_id, post_id])
            .map(|_| ())
            .map_err(|err| err.to_string())
    }

    /// Records the main post of a Story on a site, the moment the site confirmed it.
    pub fn save_publication(&self, publication: &Publication) -> Result<(), String> {
        self.save_output(publication, "main")
    }

    /// Story `story_id`'s posts on site `site_id` when published by Day,
    /// by their key (the index, or a Day's heading), in no order.
    pub fn day_posts(&self, story_id: i64, site_id: i64) -> Vec<(String, Publication)> {
        self.map_rows(
            "SELECT output_key, story_id, site_id, post_id, remote_status, size, revision, synced_at, fingerprint, changed_on_site FROM publications
             WHERE story_id = ?1 AND site_id = ?2 AND output_key != 'main' AND edition = 1",
            params![story_id, site_id],
            |row| {
                Ok((
                    row.get(0)?,
                    Publication {
                        story_id: row.get(1)?,
                        site_id: row.get(2)?,
                        post_id: row.get(3)?,
                        remote_status: row.get(4)?,
                        size: row.get(5)?,
                        revision: row.get(6)?,
                        synced_at: row.get(7)?,
                        fingerprint: row.get(8)?,
                        changed_on_site: row.get(9)?,
                    },
                ))
            },
        )
    }

    /// Keeps one post of a Story published by Day under `key`.
    pub fn save_day_post(&self, key: &str, publication: &Publication) -> Result<(), String> {
        self.save_output(publication, key)
    }

    fn save_output(&self, publication: &Publication, key: &str) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO publications(story_id, site_id, post_id, remote_status, size, revision, synced_at, fingerprint, changed_on_site, output_key)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
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
                    publication.changed_on_site,
                    key
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
    fn what_a_storys_posts_show_of_where_it_went_is_kept_per_site() {
        let (db, _, _) = library("place-choice");
        assert_eq!(db.place_choice(3, 1), PlaceChoice::default(), "nothing is shown until chosen");
        let choice = PlaceChoice { places: true, rough: true, left_out: vec![4, 9], journey: true };
        db.save_place_choice(3, 1, &choice).unwrap();
        assert_eq!(db.place_choice(3, 1), choice);
        assert_eq!(db.place_choice(3, 2), PlaceChoice::default(), "another site starts afresh");
        db.save_place_choice(3, 1, &PlaceChoice::default()).unwrap();
        assert_eq!(db.place_choice(3, 1), PlaceChoice::default());

        assert_eq!(db.journey_post(5, 1), None);
        db.set_journey_post(5, 1, 90).unwrap();
        db.set_journey_post(5, 1, 91).unwrap();
        assert_eq!((db.journey_post(5, 1), db.journey_post(5, 2)), (Some(91), None));
    }

    #[test]
    fn a_posts_record_outlives_its_story_and_never_passes_to_another() {
        let (db, _, _) = library("publication-identity");
        let site_id = db.save_site(&site("https://example.com", "ana")).unwrap();
        let trip = db.create_trip(&crate::trip::TripDraft::new("Spain", "Europe/Madrid")).unwrap();
        let story = db.create_story(trip, "Granada").unwrap();
        let publication = Publication { story_id: story, site_id, post_id: 7, remote_status: "publish".into(), size: "standard".into(), revision: 1, synced_at: 1, fingerprint: String::new(), changed_on_site: false };
        db.save_publication(&publication).unwrap();
        db.save_publication_media(story, site_id, &[41]).unwrap();

        // Archived: still the same post.
        db.set_story_archived(story, true);
        assert_eq!(db.publication(story, site_id).map(|found| found.post_id), Some(7));
        // Deleted: the record stays, as the post stays on the site, and a
        // new Story never takes it up, as Story ids aren't reused.
        db.delete_story(story).unwrap();
        assert_eq!(db.publication(story, site_id).map(|found| found.post_id), Some(7));
        let next = db.create_story(trip, "Granada").unwrap();
        assert_ne!(next, story);
        assert_eq!(db.publication(next, site_id), None);
        assert!(db.publication_media(next, site_id).is_empty());
    }

    #[test]
    fn a_new_draft_and_each_day_post_keep_their_own_post() {
        let (db, _, _) = library("publication-outputs");
        let site_id = db.save_site(&site("https://example.com", "ana")).unwrap();
        let trip = db.create_trip(&crate::trip::TripDraft::new("Spain", "Europe/Madrid")).unwrap();
        let story = db.create_story(trip, "Granada").unwrap();
        let post = |post_id: i64, revision: i64| Publication { story_id: story, site_id, post_id, remote_status: "draft".into(), size: "low".into(), revision, synced_at: revision, fingerprint: String::new(), changed_on_site: false };
        db.save_publication(&Publication { changed_on_site: true, ..post(7, 1) }).unwrap();
        // Saved as a new draft beside the post edited on the site: the new one
        // is the Story's post from now on; post 7 is never changed again.
        db.save_publication(&post(9, 2)).unwrap();
        assert_eq!(db.publication(story, site_id), Some(post(9, 2)));

        // Then by Day: an index and a post per Day, each under its own key.
        db.save_day_post("index", &post(20, 3)).unwrap();
        db.save_day_post("day:a1", &post(21, 3)).unwrap();
        db.save_day_post("day:b2", &post(22, 3)).unwrap();
        // A later save changes the same posts, and the main post stays as it was.
        db.save_day_post("day:a1", &post(21, 4)).unwrap();
        let mut days = db.day_posts(story, site_id);
        days.sort_by(|a, b| a.0.cmp(&b.0));
        let ids: Vec<(&str, i64, i64)> = days.iter().map(|(key, found)| (key.as_str(), found.post_id, found.revision)).collect();
        assert_eq!(ids, [("day:a1", 21, 4), ("day:b2", 22, 3), ("index", 20, 3)]);
        assert_eq!(db.publication(story, site_id), Some(post(9, 2)));
    }

    #[test]
    fn uploads_and_posts_are_remembered_per_site() {
        let (db, _, _) = library("publications");
        let site_id = db.save_site(&site("https://example.com", "ana")).unwrap();
        db.record_upload(site_id, "abc", "low", 41, "https://example.com/a.jpg", r#"{"width":1280}"#).unwrap();
        let details: String = db.conn.query_row("SELECT details FROM publish_media WHERE hash = 'abc'", [], |row| row.get(0)).unwrap();
        assert_eq!(details, r#"{"width":1280}"#);
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

        // The media the post uses, replaced as a whole.
        db.save_publication_media(3, site_id, &[41, 9]).unwrap();
        db.save_publication_media(3, site_id, &[41, 12]).unwrap();
        assert_eq!(db.publication_media(3, site_id), [12, 41]);

        // Each job is kept as it ended, newest first.
        let job = PublishJob { story_id: 3, site_id, started_at: 10, ended_at: 20, outcome: "cancelled".into(), post_id: None, uploaded: 2, reused: 0, size: "low".into(), revision: 4 };
        db.record_publish_job(&job).unwrap();
        db.record_publish_job(&PublishJob { outcome: "saved".into(), post_id: Some(7), ..job.clone() }).unwrap();
        let jobs = db.publish_jobs(3, site_id);
        assert_eq!((jobs.len(), jobs[0].outcome.as_str(), jobs[1].clone()), (2, "saved", job.clone()));

        // Kept as it starts, each upload counted as it lands: one cut short
        // still says what reached the site, and one that ends says how.
        let running = PublishJob { outcome: crate::publish::RUNNING.into(), uploaded: 0, ended_at: 10, ..job.clone() };
        let cut = db.record_publish_job(&running).unwrap();
        db.count_publish_upload(cut).unwrap();
        db.count_publish_upload(cut).unwrap();
        let newest = db.publish_jobs(3, site_id).remove(0);
        assert_eq!((newest.outcome.as_str(), newest.uploaded, newest.started_at), (crate::publish::RUNNING, 2, 10));
        assert!(newest.ended_at >= 10, "when the last upload landed");
        let ended = db.record_publish_job(&running).unwrap();
        db.count_publish_upload(ended).unwrap();
        db.finish_publish_job(ended, &PublishJob { outcome: "saved".into(), post_id: Some(7), uploaded: 1, reused: 3, ended_at: 30, ..running.clone() }).unwrap();
        let jobs = db.publish_jobs(3, site_id);
        assert_eq!((jobs[0].outcome.as_str(), jobs[0].post_id, jobs[0].uploaded, jobs[0].reused, jobs[0].ended_at), ("saved", Some(7), 1, 3, 30));
        assert_eq!((jobs[1].outcome.as_str(), jobs[1].uploaded), (crate::publish::RUNNING, 2), "the one cut short stays as it was");

        // A draft about to be made, kept from its earliest moment until confirmed.
        assert_eq!(db.pending_draft(3, site_id), None);
        db.set_pending_draft(3, site_id, 200).unwrap();
        db.set_pending_draft(3, site_id, 300).unwrap();
        assert_eq!(db.pending_draft(3, site_id), Some(200));
        db.clear_pending_draft(3, site_id).unwrap();
        assert_eq!(db.pending_draft(3, site_id), None);

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
