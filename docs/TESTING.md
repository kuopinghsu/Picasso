# Testing

What the automated tests cover, how to run the slower checks, and what has to be tried by hand before a release.

## Running the tests

```sh
make test                 # every unit and integration test, as CI runs them
make bench-check          # the performance limits, as CI runs them (docs/PERFORMANCE.md)
make check-docs           # links, images, make targets, scripts, and settings named in the documents
make deny                 # security advisories and licenses of the dependencies
make ci                   # all four of the above
make test T=backup        # only the tests whose name contains "backup"
make corpus CORPUS=~/corpus
```

The last line puts every photo in a folder of your own and its subfolders (HEIC, camera RAW, odd JPEGs) through the reading checks of the test corpus below; videos are skipped. Each photo must have its size read, decode upright at that size, and give a thumbnail of the same shape. The date and camera read are printed for a look, and a folder without photos fails rather than passing empty.

## What the tests cover

| Area | Where | What is checked |
| --- | --- | --- |
| Library database | `src/db.rs` | Upgrades from an old layout, an upgrade that fails halfway (nothing changes), opening twice, a library from a newer version, a reader that sees only whole writes and is never held up by one, a change stamp that moves with every write; scanning, renames and moves, the 90-day archive of removed photos, albums, edits, dates, places, unreadable files. |
| Duplicates | `src/app/dupes.rs` | Only files with the same content are grouped. |
| Backups and recovery | `src/backup.rs` | A restored backup brings back stars, captions, rotations, places, dates, albums, edits, presets, tracks, excluded folders, and archived photos; a backup that can't be written leaves nothing behind; a damaged library is kept aside, never overwritten, and a bad backup leaves it where it was; a library another program is writing to passes the integrity check, so it is never offered for replacement, and neither is one Picasso isn't allowed to read (`src/app/recovery.rs`). |
| Trash | `src/app/mod.rs` | Only files that really left are reported as trashed; a file on a disconnected drive stays in the library and is reported. |
| Formats and orientation | `src/corpus.rs`, `src/image_io.rs`, `src/raw_tiff.rs`, `src/photo_info.rs` | JPEG with each of the 8 EXIF orientations and without EXIF, PNG with transparency, TIFF, BMP, GIF, WebP: size read upright, decoded upright, thumbnails; damaged and empty files fail without crashing. |
| Editing | `src/corpus.rs`, `src/edit/` | Exports keep orientation, rotation, crop, straightening, flips, and perspective, at the size the editor reports; an edit and its undo point survive a restart; an export never replaces its original, even through a link or another letter case; an export over an older one replaces it whole and leaves no temporary file; each adjustment, curve, filter, HDR, and preset. |
| Maps and GPS | `src/geo.rs`, `src/track.rs`, `src/geotag.rs`, `src/app/places.rs`, `src/app/travel.rs`, `src/world.rs`, `src/mvt.rs` | Tile math and map sources, GPX and KML tracks, geotagging from a track with time zones, place clusters, the travel map's countries. |
| Languages | `src/i18n.rs`, `src/types.rs` | Every text in the source is translated in all ten languages with the same placeholders; Asian languages leave only names in English; Traditional Chinese keeps one word for each thing in the window, `README_zh-TW.md`, and `docs/USER_GUIDE_zh-TW.adoc`; the two READMEs, and the two user guides, have the same sections, lists, tables, and code; dates and sizes are written the way each language writes them. |
| Settings | `src/config.rs`, `src/secret.rs` | Settings move from older libraries to `settings.json`; the CARTO key is encrypted. |
| Trips and Days | `src/trip.rs`, `src/db/trips.rs` | A Trip from two folders survives a reopen. A photo can be in several Trips but in one Day of each, and adding twice keeps one member. Days can be empty, share a date, and be reordered or deleted. Suggest days keeps the user's Days and doesn't depend on the computer's time zone. A clock without an offset, or one near a daylight-saving change, goes to review. Overlapping tracks each keep their own Day. Member lists and counts agree for every filter. A removed photo stays in its Trip and Story, and a reused photo id inherits nothing from it. A moved photo relinks only when exactly one photo matches. Deleting a Trip takes only what is in it. A library from before Trips upgrades and keeps its photos. Places keep the names the user gave. Resources stay private until allowed. Reading capture times can be cancelled and resumed. |
| Stories | `src/trip.rs`, `src/db/trips.rs`, `src/app/story.rs` | Blocks read back what they store, and blocks this build can't read are kept whole. A save based on an old revision is refused. A Story made from Days follows their order and leaves out videos. Cover frames keep their shape on the photo. In the editor: Enter, Backspace and the arrow keys split, join and step through blocks; Markdown shortcuts; moving and duplicating blocks with their ids; framing the cover; Escape closing the innermost thing open. |
| Story export | `src/story_html.rs`, `src/trip_package.rs` | Export always makes a new folder and never replaces one. A photo saved again during an export stops it and leaves nothing behind. An export keeps the text and edits it started with; the next one shows the changes. A folder that refuses writing, or a write that fails partway, is an error and never a shorter page. Twenty photos export whole, with relative links only, at the size asked for, without camera details, and with the originals untouched. Among them are a photo with an edit, one turned in the library and an edited camera raw file. The page needs nothing from the network. Text and Markdown marks are escaped. Missing or unreadable photos are shown and counted, never left out. Every template changes the page but not its blocks. A cropped cover is a picture of its own while the photo stays whole. A route map carries its map credit. A Trip package carries its Trip, Stories and photos, and a damaged or hostile package is refused. |
| WordPress publishing | `src/wordpress.rs`, `src/publish.rs`, `src/gutenberg.rs`, `src/upload.rs`, `src/db/sites.rs` | Finding the site's API, including in a subdirectory and without pretty permalinks. HTTPS only, except a local site in development. Credentials never follow a redirect to another origin. Typed errors for a blocked API, a wrong password, refusals and network failures. An upload over the site's limit is named and not sent again. Reads and refused requests are retried, and a write whose answer was lost is never resent. A lost upload or draft is found by its exact name. Every block kind has its core block. Upload sizes are 2048 px at quality 85 or 1280 px at quality 80, never enlarged, keeping only how the photo was taken. A draft is changed in place. A post edited on the site is left alone unless replacing was chosen. A post becomes public only when that was confirmed. Scheduling uses the site's own clock. Day posts are made once and linked. A post's record outlives its Story. The same photo at another size is a new upload, never the larger one reused, and a photo that fails to render stops the job before anything is sent. A new draft saved beside an edited or public post never changes that post, and the new draft and each Day post keep their own post. |
| Credentials | `src/keychain.rs` | A secret goes to the system's store and comes back. Without a store it lasts the session. A store that refuses is an error, never a quiet fallback. The library keeps only a reference, naming the account and site. |
| Place names and similar photos | `src/geocode.rs`, `src/burst.rs` | Positions sent for a name are rounded to about 100 m, and a cached answer is used offline. A burst is photos close in time and alike. |
| Performance | `src/bench.rs` | Catalog queries, scanning, thumbnails, and memory, with limits in CI. |

CI runs the tests (`cargo test --release --locked`, the command `make test` runs) on Windows and Linux, each on x64 and Arm64 machines, and on macOS, and `make bench-check` on both Linux machines. It installs, starts, and removes the `.deb` on Ubuntu 22.04, and starts the AppImage on AlmaLinux 8, the oldest system it supports, and Ubuntu 24.04, for both processors. It also builds the User Guide as PDF in both languages with `make guide-pdf`, which fails when a character is in none of the fonts. Like the x64 ones, the Arm packages still need the checks below on a real machine, since CI draws without a graphics card.

## Before a release, by hand

CI can't see the screen, use a real GPU, or plug in a drive. On each platform the release supports, with the packages CI built:

| Check | How |
| --- | --- |
| Install and start | Install as the README says, start from the menu or Finder, and open About: the version and commit match the release. |
| Upgrade | Start the new version over a library of the previous release, with stars, captions, albums, edits, and places. Everything is still there; Settings kept their values. |
| Uninstall | Remove the app: `sudo apt remove picasso` for the `.deb`, `make uninstall` after `make install`, or delete `Picasso.app`, the `.exe`, or the AppImage. The library and settings stay unless you delete them. |
| First run | With no library, Picasso adds the Pictures folder and scans it. |
| Paths | Add folders whose paths have spaces, non-English letters (`相片/旅行 2024`), more than 260 characters on Windows, and a symbolic link. Photos show, open, and edit. |
| External drive | Add a folder on a USB drive, eject it, start Picasso: the photos stay in the library. Plug it in and press F5: they come back with their stars. |
| Network drive | Add a folder on a network share. It scans; F5 picks up changes. |
| Display | 100%, 150%, and 200% scaling; two monitors with different scaling, moving the window between them; light and dark appearance. Text is sharp and nothing is cut off, in English, Traditional Chinese, and German (the longest). |
| Video and sound | With FFmpeg, a video plays with sound. Without it, Picasso says what is missing and photos still work. With no sound device, video still plays. |
| Maps | Places and the Map panel load tiles. Without a network, a tile that isn't cached shows an enlarged cached tile from up to four levels farther out, or stays blank, and everything else works. |
| Keyboard | Move through the library, open, edit, and go back with the keyboard alone (**Help & Shortcuts**, F1). Tab moves through Settings and dialogs, and the focused control is visible in both themes. |
| Screen reader | With VoiceOver, Narrator, or Orca on, buttons and settings are read by name. Note what isn't. |
| Frame times | `PICASSO_FRAME_STATS=1` with a large library while scrolling: few frames over 16.7 ms (docs/PERFORMANCE.md). |
| Trip to export, offline | Make a Trip from two folders, one on a USB drive. Suggest days, write a Story with photos, a gallery and a route map, then eject the drive and restart. The photos show as unavailable, never as other photos. Plug it back in and export with the network off: the folder opens on another computer with every photo and link. |
| Publishing | On a WordPress site made for testing, never a real one: connect, save a draft at each upload size, change the post on the site and save again (Picasso stops), then publish. The site's editor opens the post without block warnings. |
| Credential store | Connect a site, quit, and start again: the site is still connected, its password read from Keychain, Credential Manager, or Secret Service. Disconnect removes it there. |

Write down the platform and version where each was tried, as the checklist in [`RELEASING.md`](RELEASING.md) asks. Passing on one platform says nothing about another.

## Trips, Stories and publishing: results

What was run for Trips, Stories and WordPress publishing, on what, and what is still to be tried. Add to it as each part changes; results for a revision stop counting once that part is changed again.

### Automated, on commit `bc1d0a6`

Run on macOS (Darwin 27.2), Apple Silicon, rustc 1.97.0, together with a one-line fix to `docs/USER_GUIDE_zh-TW.adoc` that writes `/` as code, as the English guide does.

| Check | Result |
| --- | --- |
| `make test` | 382 passed, 1 ignored |
| `make lint` | no warnings |
| `make bench-check` | within its limits; peak memory after scan and thumbnails 268 MB |
| `make deny` | advisories, bans, licenses and sources ok |
| `make check-docs`, `make guide` | ok |

CI runs the same tests on Windows and Linux (x64 and Arm64) and macOS. That builds the Windows and Linux credential stores but never opens them.

### Export failures and Trips at scale, on top of `bc1d0a6`

Run on 2026-11-01 on the same machine, an Apple M2 Max. With these tests and the later ones in the coverage table above, `make test` gives 393 passed and 1 ignored; `make lint`, `make bench-check`, `make check-docs` and `make guide` are clean.

| What | How | Result |
| --- | --- | --- |
| A photo saved again during an export | A test appends to the first photo's file as soon as the export has moved past it | The export stops with "source changed"; the parent folder is left empty, with no staging folder |
| Permission refused | A test exports into a read-only folder (skipped when run as an administrator, who can write anyway) | An error; the folder keeps only what it had |
| A write failing partway, as on a full disk | A test makes the page's file impossible to replace after the photos are written | An error, and no manifest claims the export is whole. The free-space check before an export is tested through its estimate, not on a full disk |
| 20 photos with edits and RAW | Plain landscape and portrait photos, one turned in the library, one cropped and brightened, one with a Unicode file name, and a darkened TIFF-based raw file with an embedded preview, through HTML and Markdown | Every photo shown once, every link relative and inside the folder, sizes as expected, no EXIF, originals' checksums unchanged |
| Trips at scale | `make bench`: 100,000 catalog photos and a 10,000-photo Trip over 365 Days | Every step well under its limit. The Trips view's queries take 3.2 ms; it now keeps their answers until the library changes, checked in 0.9 µs a frame. Numbers are in `docs/PERFORMANCE.md` |

### Tried by hand

| Date | Where | What was seen |
| --- | --- | --- |
| 2026-10-30 | macOS, a local WordPress site made for testing | Connected, with the password in Keychain. Chose categories and tags, and added a missing tag. An edit made on the site was found and replaced by choice. A post was published, another scheduled in the site's own time, and an index with a Day post made. The site's editor opened every block kind without validation warnings. A post with every block kind was viewed under Twenty Twenty-Five and a second travel theme, at 1280 and 375 px wide. Every photo loaded with its responsive sizes, nothing scrolled sideways, and galleries and grids kept their columns. The test posts were trashed afterwards. |
| 2026-10-30 | macOS, a copy of a real library | Suggest trips found 4 groups, and a date range narrowed them to 2. New trip's preview counted photos, videos, located photos and possible copies before anything was written. Suggest places made 25 Places at 300 m and 2 h, and 17 at 1000 m and 3 h. |
| 2026-10-31 | macOS, a copy of a real library | Similar photos found a pair taken 3 s apart, and nothing was selected, moved or starred unless asked. |

### Not run yet

- **Credential stores on Windows and Linux.** Credential Manager and Secret Service are built but have never been opened on those systems. Each needs its own desktop session.
- **An export opened on another computer with no network.** The 20-photo test checks that nothing points outside the export's folder, but no export has been copied to a second machine. The raw file in the test is made up; real camera raw files get only the reading checks of `make corpus`.
- **A really full disk.** The free-space check and a failed write are tested, but no export has been run while a disk fills up.
- **Upload bytes on a real site.** Nobody has checked the sizes of the files a site actually received at 1280 px, with RAW input, or compared checksums of the originals before and after.
- **WordPress failures on a real site,** such as a revoked password, a server error or a lost answer. They are tested against a fake site only.
- **Design review.** Trips, the Story editor and the publishing dialog haven't been compared with the Library in light and dark, at 100–200 % scaling, in English, Traditional Chinese and German, with the keyboard alone and with a screen reader. The screenshots need a library without private photos.
