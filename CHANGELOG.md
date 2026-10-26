# Changelog

All notable changes to Picasso are listed here, newest first. Each release on GitHub uses its
section of this file as the release notes.

## [0.9.5] - Unreleased

### Added

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
  or move a library, and `docs/DATABASE.md`, `docs/TESTING.md`, `docs/PERFORMANCE.md`,
  `docs/RELEASING.md`, and `docs/ROADMAP.md`.
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

### Changed

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

[0.9.5]: https://github.com/kuopinghsu/picasso/compare/v0.9.4...HEAD
[0.9.4]: https://github.com/kuopinghsu/picasso/compare/v0.9.3...v0.9.4
[0.9.3]: https://github.com/kuopinghsu/picasso/compare/v0.9.2...v0.9.3
[0.9.2]: https://github.com/kuopinghsu/picasso/compare/v0.9.1...v0.9.2
[0.9.1]: https://github.com/kuopinghsu/picasso/compare/v0.9.0...v0.9.1
[0.9.0]: https://github.com/kuopinghsu/picasso/releases/tag/v0.9.0
