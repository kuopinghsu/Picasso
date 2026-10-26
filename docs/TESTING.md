# Testing

What the automated tests cover, how to run the slower checks, and what has to be tried by hand before a release.

## Running the tests

```sh
make test                 # every unit and integration test, as CI runs them
make bench-check          # the performance limits, as CI runs them (docs/PERFORMANCE.md)
make check-docs           # links, make targets, scripts, and settings named in the documents
make deny                 # security advisories and licenses of the dependencies
PICASSO_CORPUS=~/corpus cargo test --release corpus -- --nocapture
```

The last line puts every photo and video in a folder of your own (HEIC, camera RAW, odd JPEGs) through the reading checks of the test corpus below. Each file must have its size and date read, decode upright at the size read, and give a thumbnail of the right shape.

## What the tests cover

| Area | Where | What is checked |
| --- | --- | --- |
| Library database | `src/db.rs` | Upgrades from an old layout, an upgrade that fails halfway (nothing changes), opening twice, a library from a newer version; scanning, renames and moves, the 90-day archive of removed photos, albums, edits, dates, places, unreadable files, duplicates. |
| Backups and recovery | `src/backup.rs` | A restored backup brings back stars, captions, rotations, places, dates, albums, edits, presets, tracks, excluded folders, and archived photos; a backup that can't be written leaves nothing behind; a damaged library is kept aside, never overwritten, and a bad backup leaves it where it was. |
| Trash | `src/app/mod.rs` | Only files that really left are reported as trashed; a file on a disconnected drive stays in the library and is reported. |
| Formats and orientation | `src/corpus.rs`, `src/image_io.rs`, `src/raw_tiff.rs`, `src/photo_info.rs` | JPEG with each of the 8 EXIF orientations and without EXIF, PNG with transparency, TIFF, BMP, GIF, WebP: size read upright, decoded upright, thumbnails; damaged and empty files fail without crashing. |
| Editing | `src/corpus.rs`, `src/edit/` | Exports keep orientation, rotation, crop, straightening, flips, and perspective, at the size the editor reports; an edit and its undo point survive a restart; an export never replaces its original; PNG and TIFF exports are written whole or not at all; each adjustment, curve, filter, HDR, and preset. |
| Maps and GPS | `src/geo.rs`, `src/track.rs`, `src/geotag.rs`, `src/app/places.rs`, `src/app/travel.rs`, `src/world.rs`, `src/mvt.rs` | Tile math and map sources, GPX and KML tracks, geotagging from a track with time zones, place clusters, the travel map's countries. |
| Languages | `src/i18n.rs`, `src/types.rs` | Every text in the source is translated in all ten languages with the same placeholders; Asian languages leave only names in English; Traditional Chinese keeps one word for each thing in the window and in `README_zh-TW.md`; the two READMEs have the same sections, lists, tables, and code; dates and sizes are written the way each language writes them. |
| Settings | `src/config.rs`, `src/secret.rs` | Settings move from older libraries to `settings.json`; the CARTO key is encrypted. |
| Performance | `src/bench.rs` | Catalog queries, scanning, thumbnails, and memory, with limits in CI. |

CI runs `make test` on Windows and Linux, each on x64 and Arm64 machines, and on macOS, and `make bench-check` on both Linux machines. It installs, starts, and removes the `.deb` on Ubuntu 22.04, and starts the AppImage on Ubuntu 22.04 and 24.04 and AlmaLinux 8 and 9, for both processors. Like the x64 ones, the Arm packages still need the checks below on a real machine, since CI draws without a graphics card.

## Before a release, by hand

CI can't see the screen, use a real GPU, or plug in a drive. On each platform the release supports, with the packages CI built:

| Check | How |
| --- | --- |
| Install and start | Install as the README says, start from the menu or Finder, and open About: the version and commit match the release. |
| Upgrade | Start the new version over a library of the previous release, with stars, captions, albums, edits, and places. Everything is still there; Settings kept their values. |
| Uninstall | Remove the app as the README says. The library and settings stay unless you delete them. |
| First run | With no library, Picasso adds the Pictures folder and scans it. |
| Paths | Add folders whose paths have spaces, non-English letters (`相片/旅行 2024`), more than 260 characters on Windows, and a symbolic link. Photos show, open, and edit. |
| External drive | Add a folder on a USB drive, eject it, start Picasso: the photos stay with a note. Plug it in and press F5: they come back with their stars. |
| Network drive | Add a folder on a network share. It scans; F5 picks up changes. |
| Display | 100%, 150%, and 200% scaling; two monitors with different scaling, moving the window between them; light and dark appearance. Text is sharp and nothing is cut off, in English, Traditional Chinese, and German (the longest). |
| Video and sound | With FFmpeg, a video plays with sound. Without it, Picasso says what is missing and photos still work. With no sound device, video still plays. |
| Maps | Places and the Map panel load tiles. Without a network, missing tiles stay blank and everything else works. |
| Keyboard | Move through the library, open, edit, and go back with the keyboard alone (**Help & Shortcuts**, F1). Tab moves through Settings and dialogs, and the focused control is visible in both themes. |
| Screen reader | With VoiceOver, Narrator, or Orca on, buttons and settings are read by name. Note what isn't, for [`ROADMAP.md`](ROADMAP.md). |
| Frame times | `PICASSO_FRAME_STATS=1` with a large library while scrolling: few frames over 16.7 ms (docs/PERFORMANCE.md). |

Write down the platform and version where each was tried, as the checklist in [`RELEASING.md`](RELEASING.md) asks. Passing on one platform says nothing about another.
