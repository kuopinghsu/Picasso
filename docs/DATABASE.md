# The library database

Picasso keeps its library in one SQLite file, `picasso.db`, in the app data directory (README → Data). This page describes what is in it, how its layout is versioned, and how an older library is brought up to date. The code is `src/db.rs`.

The photos themselves are never in the database. It holds what Picasso learned from them and what you added: stars, captions, albums, edits, places, and the list of folders.

## Files

| File | Where | What | Safe to delete |
| --- | --- | --- | --- |
| `picasso.db` (with `-wal` and `-shm` while open) | data directory | The library | No: stars, captions, albums, edits, and folders are only here |
| `backups/picasso-YYYYMMDD-HHMMSS.db` | data directory | Weekly and manual backups of `picasso.db`, the five newest | Yes, but they are your way back |
| `picasso-damaged-YYYYMMDD-HHMMSS.db` | data directory | A library that couldn't be opened, kept aside by the recovery window | Once you no longer need it |
| `track-recovery.json` | data directory | Unsaved edits of the open GPS track, kept a few seconds after each change and removed once saved or discarded; offered back at the next start after a quit or a crash | Yes: the edits are lost |
| `settings.json` | settings folder | Choices in **Settings** (language, theme, map, export, updates) and the ten most recent searches | Yes: everything goes back to defaults |
| `thumbs.db` | cache directory | Thumbnails, and the thumbnails of edited photos | Yes: made again |
| `hashes.db` | cache directory | File hashes of the last duplicate search | Yes: computed again |
| `tiles/` | cache directory | Downloaded map and terrain tiles | Yes: downloaded again |
| `.picasso-geotags.json` | each photo folder you geotagged | Places set by **Geotag from GPS track**, beside the photos | Then the places stay only in the library |
| `.picasso-renames.json` | each photo folder you renamed files in | The old and new names of the last 50 renamings by **Rename files…**, newest last, removed once all are undone | Yes: **Undo** can no longer put the names back |

Picasso also reads the `.picasa.ini` files Picasa left in photo folders, for stars, captions, and rotations, and never writes them.

## Tables

| Table | One row per | Notes |
| --- | --- | --- |
| `folders` | Folder you added | `scan_pending` is 1 until a scan of it reaches the end, so an interrupted scan resumes on the next start. |
| `photos` | Photo or video found in those folders | What the scan read from the file (`width`, `height`, `bytes`, `taken_at`, `modified_at`, `camera`, `lens`, `focal_mm`, `gps_lat`, `gps_lon`, `duration`) and what you set (`starred`, `caption`, `rotation`, `pin_lat`, `pin_lon`, and `taken_at` with `date_set` = 1). `file_taken_at` keeps the file's own date when you change it. `indexed_version` is the `INDEX_VERSION` that read the row; a lower one is read again on the next scan. |
| `albums`, `album_photos` | Album, and photo in an album | |
| `edits` | Edited photo | `steps` is the edit history as JSON, `applied` how many of them are in effect, so Undo and Redo survive a restart. |
| `photo_archive` | Photo that left the library in the last 90 days | Its stars, caption, rotation, place, date, albums, and edits as JSON, keyed by path, size, modification time, and date taken, so they come back if the file does. |
| `unreadable` | File or folder a scan couldn't read | With the reason. A file is tried again when it changes, or after 7 days. |
| `excluded` | Folder left out of scanning | |
| `tracks` | GPX or KML file | `points` is the track in the compact form of `Track::encode`. |
| `settings` | Value kept with the library | `user_presets` (your edit presets), `side_panel`, `viewer_info`, and `seeded` (the Pictures folder was added on the first start). Settings of older versions that now live in `settings.json` are read from here once. |
| `id_floor` | | The highest photo id ever used, kept by a trigger, so an id is never given to another photo and nothing cached under it is shown for the wrong one. |
| `media_refs` | Photo a Trip or Story uses | Its catalog row in `photo_id` while it is in the library, emptied when the row goes, and the path, file name, size, modification time, and file date to find it again; `unlinked_at` is when the row went. Kept as long as anything uses it, past the 90 days of `photo_archive`. Triggers keep the hints current. |
| `trips` | Trip | Title, description, destination, optional `first_day` and `last_day` (`YYYY-MM-DD`), `zone` (a named zone such as `Asia/Taipei`, or an offset such as `+08:00`), `cover_ref`, and `archived`. |
| `trip_days` | Day of a Trip | `position` orders them; `day` is an optional date, and several Days may share one. A Day never moves to another Trip. |
| `trip_media` | Photo or video in a Trip | `position` is the order they joined; `day_id` is a Day of the same Trip, which triggers enforce, with `day_position` its order there and `day_source` who chose it (0 none, 1 suggested, 2 the user). |
| `trip_tracks` | GPS track linked to a Trip | `track_id` while the track is in the library; the path and name stay when it goes. |
| `stories`, `story_blocks`, `story_media` | Story, its blocks, and the photos in them | `revision` goes up with each save, so a stale editor can't overwrite. Each block has a `uid` that stays through saves, a `kind`, and a JSON `payload` in the layout of its `version`; a block from a newer Picasso is kept unchanged. Each photo in a block has its own caption and alt text. |

Indexes cover the orders and lookups the window uses: by folder, by date taken and name (the default order), by name, by size (duplicates), by star, and album membership.

## Versions

`PRAGMA user_version` holds the layout version, `SCHEMA_VERSION` in `src/db.rs`.

| Version | Picasso | Change |
| --- | --- | --- |
| 0 | up to 0.9.4 | Libraries made before the version was kept. They are recognized by their tables and columns and upgraded in place. |
| 1 | 0.9.5 | The version is kept. |
| 2 | 0.9.6 | Trips and Stories: `media_refs`, `trips`, `trip_days`, `trip_media`, `trip_tracks`, `stories`, `story_blocks`, `story_media` ([TRAVEL.md](TRAVEL.md)). |

## How a library is upgraded

Opening a library runs `Database::migrate` before anything else:

1. It takes the write lock and starts one transaction.
2. It creates the tables and indexes that are missing, adds the columns that are missing (each with a default, so existing rows stay valid), and drops the tables of removed features (`faces`, `people`). Every step checks what is there first, so running it again changes nothing.
3. It sets `user_version` to `SCHEMA_VERSION`, unless the library is from a newer Picasso.
4. It commits.

A step that fails, or a crash or power loss before the commit, rolls everything back: the library is left exactly as it was and can be opened by the version that made it. If the library can't be opened at all, Picasso shows a recovery window instead of starting (README → Data).

A library from a newer Picasso opens too, and keeps its number. Changes to the layout only ever add tables and columns, which an older build ignores, so a library can go back and forth between versions.

### Changing the layout

- Add tables with `CREATE TABLE IF NOT EXISTS` and columns with `ensure_column`, giving every new column a default. Never rename or remove a column, and never change what an existing one means: an older Picasso still reads it.
- Data that has to be converted goes in `upgrade()` as an `UPDATE` that does nothing when run twice.
- Raise `SCHEMA_VERSION`, add a row to the table above, and add a test beside `upgrades_an_older_library` that starts from the previous layout.
- When the scanner reads something new from files, raise `INDEX_VERSION` instead, so existing rows are read again on the next scan.

The tests in `src/db.rs` and `src/db/trips.rs` cover an upgrade from version 1, an upgrade from an old layout, one that fails halfway (the library is unchanged and its version stays 0), opening a library twice, and opening one from a newer version.
