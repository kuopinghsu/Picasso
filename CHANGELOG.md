# Changelog

All notable changes to Picasso are listed here, newest first. Each release on GitHub uses its
section of this file as the release notes.

## [0.9.6] - Unreleased

### Added

- Suggest days and Suggest places show how many photos' times they have read, with
  Cancel; times already read are kept, so the next run reads only the rest.
- Frame a Story's cover: choose a shape, fixed or free to drag to any shape, and the
  part of the photo it shows, for that Story only. Preview, export and WordPress use the framed cover; the photo, its edits
  and its other uses stay whole. The Story photo picker can show only starred photos
  or those not in the story yet, and marks photos already used.
- The user guide, in English and Traditional Chinese, has a Trips and Stories chapter:
  making Trips, Days and reviewing times, Places, tracks and resources, Trip search
  words, Similar photos, writing Stories, templates, preview and export, publishing to
  WordPress, and moving, archiving and deleting Trips.
- Suggest trips… on the Trips page groups the library's photos, or those taken between two dates, by the days they were taken,
  with each group's folders, cameras and locations, and camera clocks that were off can be moved for the grouping, and makes a Trip of one through New trip.
- Places in a Trip: Suggest places groups its located photos by distance and time, and each
  Place can be named, in the local language too, given notes, reordered, and shown on its own;
  named Places stay when suggesting again. Photos without GPS are placed by the Trip's tracks,
  in its time zone, without pinning them.
- Edit place can look up a Place's name online, once Look up place names online is turned on in
  Settings → Map. Only the position, rounded to about 100 m, is sent to OpenStreetMap Nominatim,
  only when you press Look up name, and answers are kept on this computer.
- Resources in a Trip: keep documents, links, notes and track files with it, open them from the
  Trip, and locate a file that moved. Each stays private unless you allow it to be published.
- A Trip's photos can be narrowed to those without a caption, and the Library search finds
  photos in no Trip with is:notrip, or in one with is:intrip.
- Search finds photos by Trip, Day, Place and Story with trip:, day:, place: and story:, and
  those whose Trip time needs review with is:review.
- Stories can be shown as a Photography Journal, Travel Diary, Travel Guide (with a list of
  sections) or Photo Essay, picked in the Story's header; the preview and export follow it.
- Practical info and Link blocks in Stories, in exports and WordPress posts; a Link can be filled
  from the Trip's links that may be published. Export and publishing warn about small photos,
  and export about a Story without a cover.
- Similar photos… in a Trip suggests groups of photos taken close together that look alike, such
  as bursts, with adjustable time and likeness; a group can be selected, and nothing is deleted
  or starred.
- Adding photos to a Trip first says how many have a location, are in another Trip, have no
  date, or may be copies of another.
- Publish a Story to your own WordPress site from the Story's ⋯ menu. Connect sites in
  Settings → Publishing with an Application Password, kept in the system's password store.
  The dialog shows the site, the post, categories and tags read from the site, new ones added there from the dialog, the photo size
  (Standard web or Low resolution), whether to keep camera details, a photo preview and the
  post's state. Photos never carry their location; each new upload carries its description and is named for the Story and the photo's file.
- A Story is saved as a draft unless you choose to publish it; publishing first confirms what
  readers will see, at once or at a date and time in the site's time zone. A Summary in the dialog lists what saving will change on the site. A Story with Days can go out as an index post and one post per Day, linked to each other. Later saves update the same post. A post edited on the site is never
  overwritten without your choice, and a post Picasso has no record of is found before another
  is made.
- Uploads of the same bytes are reused, interrupted or lost requests are recovered without
  duplicates, safe requests are retried with backoff, and each job is kept under History.

### Changed

- Local macOS builds are signed with a self-signed `Picasso Development` certificate that
  `make sign-identity` makes once; release builds from CI stay signed ad hoc.
- Reorganized the English and Traditional Chinese READMEs around the project overview,
  features, installation, common workflows, data storage and development. Simplified the
  English wording for readers who use English as an additional language.
- Updated both user guides with clearer navigation, separate export and GPS track workflows,
  supported formats, storage locations, backup coverage and troubleshooting steps.
- Expanded the release guide with preparation and verification steps, the complete release
  file list, checksum commands, macOS signing configuration and recovery from failed releases.
  Pushed tags are preserved; corrections that change release contents use a new patch version.

### Fixed

- Renaming, deleting or moving a photo or a folder of photos on disk now shows in the library
  and the folder list while Picasso is open, as soon as the scan lists the change and while it
  goes on reading new photos: moved and renamed photos keep their stars, captions, albums, edits, Trips and Stories, and photos of a
  deleted folder leave the library. An added folder renamed in place is followed under its new
  name, and one deleted or moved away leaves the folder list. A folder on an unplugged drive
  still keeps its photos.
- A zoom lens has one name at every focal length: Hasselblad's "XCD 35-100E@38" and
  "XCD 35-100E@100" are both XCD 35-100E in Statistics and search; the focal length
  stays in Focal lengths. Names already in the library are tidied when it opens.
- One camera now has one name whichever program wrote the file: "NIKON CORPORATION
  NIKON D850" and "Nikon D850" are both Nikon D850 in Statistics, the info panel
  and search. Names already in the library are tidied when it opens.
- Statistics › Cameras and Lenses: opening Other shows up to 12 names at once, with
  the rest scrolling in the list, and the top names show their photo count beside
  the percentage, as the rest do.
- Standardized Traditional Chinese README terminology to match the interface, fixing the
  terminology test that blocked CI on Windows, Linux and macOS.
- Empty library views now offer actions for the current context: clear a search, remove the
  edited filter, return to the library, or rescan a folder. Import is offered for an empty library.
- Star and Unstar in the thumbnail menu apply to the selected photos and show their count.
  Toolbar star actions now follow the primary photo consistently for mixed selections.
- Shared icon buttons provide translated accessibility names and their enabled and selected states.
- The edited-only filter is separate from sorting, remains visible in the browse toolbar, and
  supports every sort order. Statistics and Travel Map clear this filter when opened.
- Clarified documentation about network access, default update checks, original-file handling,
  export filename behavior and the limits of damaged-library recovery.
- Corrected the release guide: a manual workflow run targeting a version tag can publish a
  release. Manual runs targeting branches produce build artifacts without publishing.

## [0.9.5] - 2026-10-27

### Added

- A track editor for GPX, KML, and Polarsteps tracks. A folder's tracks are listed at the top of
  the library grid under **GPS tracks**, each with the shape of its path, its format, and its
  distance; clicking one, or **Edit track** on the right-click menu there or in Places, opens it.
  The editor cleans the track up the way GPXViewer does: it drops points reached faster than the
  speed limit, removes redundant points, merges the points of a stay, and can fill long gaps with
  a curve. The settings and their defaults match GPXViewer's and are in **Settings → GPS tracks**.
  The map fills the window, with the tools, the views (whole track, start, end, highest point),
  and the zoom floating over it, a bar of actions for the selected points, and messages that fade
  away. The panel beside it shows the statistics as tiles, an elevation profile linked to the map,
  and the selected point, with ← and → to step from point to point.
  Points can be moved, added, and removed, singly or with a box selection, with undo and redo;
  flights are drawn dashed and stays get a circle. **Help & Shortcuts** lists the editor's keys
  and opens over it. **Save as GPX…** and **Save as KML…** save a
  copy beside the track, named `-optimized`, with the settings and how long each stay lasted
  inside, so it opens as saved; tracks saved by GPXViewer do too. The original file is never
  changed, and Save won't write over it. Undo going back over a cleanup puts its settings back.
  Unsaved edits are kept a few seconds after each change, so if Picasso quits or stops
  unexpectedly, they are offered back at the next start.
- **Look up heights** in the track editor fills in the heights of a track that has none, such as
  one from Polarsteps, from the free Terrain Tiles on AWS. Only the numbers of the tiles along the
  track are sent, as when showing the map, and a long track is read from coarser tiles so a lookup
  downloads at most about 35 MB. The tiles are kept with the map tiles; points whose tile failed
  can be looked up again, and undo takes the heights off.
- Polarsteps location exports, such as `locations.json`, are read as GPS tracks whatever the
  `.json` file is named. Other JSON files are skipped, and the ones Google Takeout writes beside
  each photo aren't opened at all.
- Places and the Map panel zoom three levels closer, to about 30 m across a hundred points, and
  show a scale bar in the bottom-left corner. Where a map source has no tiles that close, such as
  Esri imagery in some places, the closest tiles it has are enlarged instead of showing a "Map data
  not yet available" picture. Clicking a cluster whose photos were taken at one spot selects them
  rather than zooming further.
- A zoom bar between the + and − buttons of Places and the Map panel, with a stop for each zoom
  level: click or drag along it to go straight to a level. The control fades into the map until
  the pointer comes near it, and shows at full strength while you zoom with a two-finger pinch or
  the wheel.
- Two more CARTO map styles: **Voyager**, colorful and detailed, and **Dark Matter**, a dark map
  that suits the dark theme. **Settings → Map** lists CARTO once, with a style picker for Voyager,
  Positron, and Dark Matter, which share one API key.
- Packages for 64-bit Arm: `picasso-…-windows-arm64.exe` and `.zip` for Windows on Arm, and
  `picasso-…-linux-arm64.AppImage`, `.deb`, and `.tar.gz` for Arm Linux. They are built and tested
  on Arm machines.
- When the library can't be opened, for example after a disk error, Picasso shows a window
  instead of closing: **Try again**, **Restore the backup of …** (the newest one that can be
  read), **Start a new library**, **Open library folder**, or **Quit**. The library that couldn't
  be opened is never overwritten; it stays beside the new one as `picasso-damaged-….db`. Restoring
  and starting over are offered only when the library file itself is damaged, not when it is only
  busy or Picasso isn't allowed to read it.
- **About** and `picasso --version` show the commit a build was made from, and the release notes
  name it too.
- Every package except the bare `.exe` includes `THIRD-PARTY-NOTICES.txt` with the licenses of
  the libraries built into Picasso; the AppImage also carries the licenses of the system libraries
  it bundles. `make notices` writes the file. Each release also carries the source of rawler, the
  RAW decoder, as its license asks.
- The Mac app is signed with a Developer ID and notarized by Apple when the project's signing
  secrets are set up; the release notes say which.
- Documentation: a Quick start at the top of the README, what a backup holds and how to recover
  or move a library, and `docs/DATABASE.md`, `docs/TESTING.md`, `docs/PERFORMANCE.md`, and
  `docs/RELEASING.md`.
- A User Guide in English and Traditional Chinese, `docs/USER_GUIDE.adoc` and
  `docs/USER_GUIDE_zh-TW.adoc`, that walks through Picasso from installing it to recovering a
  library. `make guide` builds each as one HTML file that opens offline, and `make guide-pdf` as
  PDF. Each release carries both PDFs, `picasso-…-user-guide.pdf` and
  `picasso-…-user-guide-zh-TW.pdf`.
- For developers: `make bench` measures scanning, thumbnails, search, and memory on a made-up
  library, and `make bench-check` fails when they pass the limits in `docs/PERFORMANCE.md`.
  `PICASSO_FRAME_STATS=1` prints how long frames take. `make deny` checks the dependencies for
  security advisories and licenses, and `make check-docs` checks links, `make` targets, and
  settings named in the docs. CI runs all three, and installs, starts, and removes the `.deb`.
- `make help` lists the Makefile's targets. New ones: `make ci` runs the checks CI runs, `make test
  T=name` runs only matching tests, `make install` and `make uninstall` put Picasso under
  `~/.local` on Linux, and `make debug`, `make check`, `make lint`, `make run-stats`,
  `make corpus`, `make world-map`, and `make version`.
- Ctrl+C, or **Copy picture** on a thumbnail's right-click menu or the viewer's ··· menu, puts the
  photo on the clipboard with its edits, at up to 4096 px. **Copy path** copies the paths of the
  selected photos, and **Open with default app** opens the file in the system's app for it.
  Ctrl+F jumps to the search box.
- Search words: `type:video`, `type:photo`, `type:raw`, or a format such as `type:jpg`;
  `gps:yes` or `gps:no`; `is:edited` or `is:starred`; `camera:` and `lens:`; `focal:35` or
  `focal:24-70`; and `mp:20-` for the pixel size. A word starting with `-`, such as `-beach` or
  `-type:video`, leaves out what it would find.
- The search box lists your ten most recent searches when you click in it, narrowed to the ones
  that contain what you type; click one, or pick it with ↑ and ↓ and press Enter, to search
  again. They are kept in `settings.json`, and **Clear recent searches** at the bottom of the list
  forgets them.
- Places follows the search, showing and framing only the matching photos, and the search stays
  when going from the Library to Places and back. **Starred only** in its toolbar leaves only the
  starred ones. Clicking a track selects the photos taken along
  it, within 300 m of the path and 14 hours of its times.
- The slideshow fades from one photo to the next and waits for the next to load. **Shuffle**
  (Shift+S) plays the photos in a random order, each once before any comes again.
- **Rename files…** on a folder's right-click menu renames the photos and videos in it from a
  pattern of exiftool keywords, `${DateTimeOriginal}%-03c.%le` with the date format
  `%Y.%m.%d-%Hh%Mm%Ss` by default; click a keyword to add it. A time offset such as `+1:30`
  moves the dates in the names for a camera whose clock was off. Every new name is shown before
  anything changes, a file already named by the pattern keeps its name, no file is renamed over
  another, and if one can't be renamed none are. The photos keep their stars, captions, albums,
  edits, and places, and their `.xmp`, `.aae`, and `.thm` files are renamed with them. Each
  renaming is listed in `.picasso-renames.json` in the folder, so **Undo** puts back earlier
  names one renaming at a time, also after Picasso was closed; a file whose earlier name another
  file has now keeps its name and stays listed.

### Changed

- A new color theme in travel-photo tones: a lagoon-teal accent, warm paper tones in the light
  theme, and a night-sea charcoal in the dark theme, with the same steps in both and text kept
  readable in each. The Travel Map has an aqua sea and stone-colored land, and places with photos
  run from pale gold through amber to burnt amber. The OpenFreeMap sea on Places is the same aqua,
  and places are marked in one coral red on the sidebar, the map pin, and Statistics.
- The bottom bar has one side panel button instead of separate Info and Map buttons; the panel
  opens on the tab it last showed, and I does the same. The thumbnail size slider shows only in
  the views with thumbnails, so not in Places, Statistics, or the Travel Map, which also have no
  side panel button.
- The CartoDB Positron map is now called **CARTO Positron**, after the company's current name.
- **Remove** in **Settings → Folders** and on the folder tree asks first, and says the files stay
  where they are.
- The library records its layout version, and an upgrade from an older version happens in one
  step: if it is interrupted, the library stays as it was and the upgrade runs again next time.
- Sizes use a decimal comma in German, Spanish, French, Italian, and Portuguese, such as 1,5 MB,
  and French units in French, such as 1,5 Mo.
- The Traditional Chinese window calls the library 圖庫 everywhere.
- A release can no longer be published twice under one version, and is published only when all
  twelve packages are there; a problem found later is fixed in the next version.
- Dependencies updated for security advisories in rustls and webpki.

### Fixed

- A large selection, such as the photos along a track in Places, filled the photo tray and pushed
  the buttons at the right of the bottom bar out of the window. The tray now shows as many
  thumbnails as fit, with the count of the rest.
- Removing a folder lost the stars, captions, albums, and edits of its photos at once. They are
  now kept for 90 days, and come back when the folder is added again, even from a new place.
- Making thumbnails held on to about 1 MB of memory for each photo, 4.7 GB for 2,000 photos,
  because the JPEG decoder started threads for every photo. The same run now peaks at 126 MB.
- **Move to trash** counted photos as trashed when their drive was disconnected, so they left the
  library although the files weren't touched. They now stay, with a note to connect the drive, and
  files the trash refuses are reported instead of being counted as trashed.
- An export to PNG or TIFF that failed partway could leave half a file under the final name; like
  JPEG, it is now written in full before it takes the name. Saving an edited copy over the original
  photo through a link or a name in another letter case is refused, as saving to the same path was.
- A backup that couldn't be written left a partial file in the `backups` folder.

## [0.9.4] - 2026-10-25

### Added

- **Statistics** in the sidebar: how many photos and videos there are and how much space they
  take, photos per year with the busiest day, and charts by month, weekday and hour of day. It also
  shows where photos were taken (how many have a location, the top places, and the GPS track
  distance) and which cameras, lenses and focal lengths were used. Clicking a year, the busiest
  day, a camera or a lens shows those photos in the Library; clicking a place shows it on the map.
  The Cameras and Lenses cards list the eight most used; clicking **and N more** lists the rest
  in a scrolling box, and **Show fewer** puts it away. With a search typed in, the page counts only
  the matching photos.
- **Places clusters:** nearby photos merge into one circle sized by how many photos it holds, and
  zooming in splits it into smaller groups and finally single photos. Hovering a circle shows a
  card with the date, folder, count and thumbnails, plus **Open**, **Select these photos** and
  **Zoom in**. Clicking a circle zooms to its photos. New **+** and **−** buttons zoom the map.
- **In this area:** on the Places page the right panel lists the places and GPS tracks the map is
  showing, largest first, with their dates, folders and thumbnails. Hovering a row highlights it on
  the map and the other way round, and clicking a row zooms to it.
- **Travel Map** in the sidebar: a globe or flat world map that colors every country, and the
  states and provinces of the largest countries, where geotagged photos were taken. Places with
  more photos are a deeper color. The side panel counts countries, continents and the next goal,
  shows coverage by continent and lists your countries; clicking a country or a colored place
  shows its photos on the Places map. Statistics has a Travel card with the same totals. The map
  follows only the photos' GPS tags; nothing is marked by hand. Borders come from the public-domain
  [Natural Earth](https://www.naturalearthdata.com) data, built by `scripts/world-map.py`.
- The scan reads each photo's **lens** and **focal length** (35 mm equivalent when the file records
  it). Search also finds photos by lens name. Existing libraries read this once on the next scan.
- Traditional Chinese README (`README_zh-TW.md`).
- **Geotag from GPS track:** right-click a folder and choose **Geotag from GPS track…**, or run
  `picasso geotag <track> <folder> [--offset +01:00] [--dry-run]`. Each photo in the folder and
  the folders below it gets the place a GPX, KML or KMZ track was at when the photo was taken,
  along the track between points up to 30 minutes apart. Photos with GPS from the camera, and
  photos pinned by hand, keep their place.
  The places go to the library and to a `.picasso-geotags.json` file in each folder; the photo
  files stay as they are, and a later scan of a new library reads the places back. The camera
  time zone is optional: without it, each photo's recorded UTC offset is used, else this
  computer's time zone. Videos, and photos that record no capture time and so go by the file's
  modification time, already carry an exact moment and use it as it is. Running again with the
  same track replaces its earlier places.

### Changed

- Thumbnails grow to 320 points, up from 220. A tile larger than the cached thumbnail gets a
  sharper copy decoded for it, which the window keeps in memory, so the thumbnail cache stays
  the same size.
- The Travel Map fills the whole view, with its controls and side panel floating over the map. The
  panel slides away from its header, and a pill brings it back.
- The map moves smoothly. Clicking a country flies to it, a flick keeps the map gliding, the zoom
  buttons ease in and out, and Reset view flies home. The globe glows, has soft shading and turns
  slowly for about 20 seconds after the map opens, until you touch it.
- Clicking a place selects it and shows a card with its photo count, dates, the years you went,
  how many of its states and provinces you've seen, and a Show photos button.
- Year chips above the map show one year of travel at a time. The panel has a progress ring toward
  the next goal, continent chips that filter the country list, and a search box. Each country
  shows the years you went and a bar for its share of your photos.
- Statistics has a new look. A header sums up the library with a sparkline of photos per year, and
  year chips show the whole page for one year at a time. Tiles with icons compare each number with
  the year before. Cover photos mark the busiest day, the longest streak, the busiest month and the
  first photo. An activity calendar shows each day of the year; clicking a day shows its photos.
  Charts have gridlines, grow in when the page opens and show a card on hover, and a grid shows the
  weekdays and hours you take photos. Cameras, lenses, photos and videos, and photos with a
  location are shown as rings. Places are a mosaic of large covers, and the Travel card follows the
  chosen year.
- Release packages include `CHANGELOG.md` and the Traditional Chinese README, and the GitHub release
  notes come from this file.

### Fixed

- HDR HEIF photos from the Hasselblad X2D II showed no camera, lens or focal length, so Statistics
  left them out of Cameras and Lenses. These files attach their EXIF to the gain map rather than
  to the picture; the scan now finds it there. HEIC files already in the library that have no
  camera are read again once on the next scan.

## [0.9.3] - 2026-10-24

### Added

- **Timeline beside the library grid:** years sit where their photos are, months are ticks, and a
  handle marks the part in view. Pointing at it names the month and its photo count; a click
  glides to that month and dragging scrubs through the library.
- The date heading of the day in view stays pinned at the top of the grid while you scroll.
- A check circle on each tile adds the photo to the selection or removes it.
- Pinch on the trackpad to zoom the Places map and the Map panel.
- The Library opens at the photo that was at the top when Picasso last closed.
- `make appimage` builds the Linux AppImage locally, with the same script CI uses.

### Changed

- Grid rows fill the window's width, and file names show over a tile on hover instead of under
  it. Sorting by name still writes them under the tiles.
- Selection actions in the library, the viewer's zoom, rotate and Edit buttons, and the editor's
  undo, redo and compare controls are compact icons, with less used commands in a ··· menu.
- Thumbnails of photos edited with crop, straighten, perspective, corners or flip are rendered from
  the original, so they are as sharp as the others. Edited thumbnails are saved in the thumbnail
  cache and replaced whenever the edit changes, including after undo and Undo all.

### Fixed

- Clicking the filmstrip while editing no longer makes it jump or slide sideways.
- The Info and Edit panels keep their layout while the next photo loads instead of flashing.
- Esc closes an open menu or drop-down without also closing the viewer.

## [0.9.2] - 2026-10-23

### Added

- **GPS tracks:** `.gpx` and `.kml` files in your folders are read by the scan and drawn as colored
  paths under the photo pins, in Places and on the Map panel. Hover a track for its name, date,
  distance and duration, and double-click it to zoom to it. A **Tracks** checkbox turns them off.

## [0.9.1] - 2026-10-23

### Added

- **Duplicates** in the sidebar finds files with identical content and offers the extra copies for
  the trash.
- **Export…** saves the selected photos with their edits as JPEG, PNG or TIFF, at full size or a
  set long edge, in the background.
- **Adjust date…** shifts or sets the date taken of the selected photos without changing the files.
- **Undo** for two minutes after **Move to trash** (Windows and Linux).
- **Transform** tool and sliders: turn a photo by any angle, tilt it in perspective, and drag its
  corners and sides; **HDR** tone mapping with five methods.
- **Map sources** in Settings: OpenFreeMap Liberty (the default, drawn in the app language),
  OpenStreetMap, CartoDB Positron and Esri World Imagery.
- **Backups** of the library, made regularly and restorable from Settings → Storage.
- **Update check** once a day, with **Check now** in Settings.
- **Excluded folders**, **Sort by Name, edited**, and search by date (`2019`, `2019-05`,
  `2019-05-17`).
- Files a scan can't read are listed from the status bar, with **Show in folder** and **Move to
  trash**.
- A photo's stars, caption, albums, pin and edits come back for 90 days if the file returns, even
  under another name.
- The viewer decodes the full-resolution photo when zoomed past the screen size.
- System fonts are used as a fallback for text the built-in fonts can't show.
- **Release packages:** a universal macOS app (Apple silicon and Intel) in a `.dmg` and a `.zip`,
  and a Linux AppImage tested on Ubuntu and RHEL-compatible systems.

## [0.9.0] - 2026-10-22

First release.

- Folder library with a date-grouped thumbnail grid, search, star filter and sorting, a folder tree
  with photo counts, albums, starred photos, and Places, a map of every photo with GPS.
- Info and Map panel beside the grid, and a viewer with zoom, pan, filmstrip, rotate, slideshow and
  video playback (with FFmpeg).
- Photo Tray for working on a selection, albums, captions and stars, and import from
  `.picasa.ini`.
- Background scan with a folder watch, JPEG, PNG, GIF, WebP, BMP, TIFF, HEIC/HEIF, camera RAW and
  videos.
- Non-destructive editor: light, tone curve, color mixer, color grading, detail, crop, masks,
  presets and export.
- Light, dark or system theme, ten interface languages, and Help & Shortcuts.
- Packages for Windows (`.exe` and `.zip`) and Linux (`.deb` and `.tar.gz`).

[0.9.6]: https://github.com/kuopinghsu/picasso/compare/v0.9.5...HEAD
[0.9.5]: https://github.com/kuopinghsu/picasso/compare/v0.9.4...v0.9.5
[0.9.4]: https://github.com/kuopinghsu/picasso/compare/v0.9.3...v0.9.4
[0.9.3]: https://github.com/kuopinghsu/picasso/compare/v0.9.2...v0.9.3
[0.9.2]: https://github.com/kuopinghsu/picasso/compare/v0.9.1...v0.9.2
[0.9.1]: https://github.com/kuopinghsu/picasso/compare/v0.9.0...v0.9.1
[0.9.0]: https://github.com/kuopinghsu/picasso/releases/tag/v0.9.0
