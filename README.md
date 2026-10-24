# Picasso

![Picasso](images/banner.jpg)

A fast local photo library, viewer, and editor in the spirit of Picasa. It watches your folders, builds a date-grouped thumbnail grid, and edits photos without touching the originals. Everything runs on this computer, and nothing is uploaded.

![The Picasso library: folder tree on the left, thumbnail grid in the middle, and photo details on the right](images/picasso.png)

![The Picasso library: image editor](images/editor.png)

## Prerequisites

- [Rust](https://rustup.rs) 1.95 or newer (`rustup update stable`)
- A C compiler, because SQLite is built from source
- [FFmpeg](https://ffmpeg.org) (`ffmpeg` and `ffprobe`) for videos. It is optional: without it, photos work and videos are skipped until it is installed.
- An internet connection for the map tiles in Places and the Map panel. Everything else works offline.

| OS | C toolchain | FFmpeg |
| --- | --- | --- |
| Windows 10/11 | [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) with "Desktop development with C++" (it includes the Windows SDK, whose `rc.exe` embeds the app icon) | `winget install Gyan.FFmpeg` |
| macOS 11+ | `xcode-select --install` | `brew install ffmpeg` |
| Debian / Ubuntu | `sudo apt install build-essential pkg-config libasound2-dev libxkbcommon-dev libgl1-mesa-dev` | `sudo apt install ffmpeg` |
| Fedora | `sudo dnf install gcc pkgconf-pkg-config alsa-lib-devel libxkbcommon-devel mesa-libGL-devel` | `sudo dnf install ffmpeg` (RPM Fusion) |

Picasso looks for FFmpeg in the `PICASSO_FFMPEG` environment variable (a folder or the `ffmpeg` executable), next to the app, on `PATH`, in WinGet's install folder, and in Homebrew's folders. After installing FFmpeg with WinGet, open a new terminal so `PATH` picks it up. Picasso also finds it without that.

On Linux, the folder picker uses the XDG desktop portal (`xdg-desktop-portal` plus the GTK or KDE backend), which most desktops already have.

## Run

```bash
cargo run --release
cargo run --release -- ~/Pictures ~/Photos
```

The first launch also adds your Pictures folder when it exists. Later launches rescan every folder you have added. Debug builds are usable, but scanning, thumbnails, and editing are much faster with `--release`.

Picasso draws with OpenGL. On WSL that uses the desktop GPU. Set `PICASSO_RENDERER=wgpu` to use Vulkan instead.

## Releases

GitHub Actions (`.github/workflows/ci.yml`) builds and tests Picasso on Windows, Linux, and macOS for every push to `main` and every pull request. Each run keeps these packages as build artifacts:

| Package | Contents |
| --- | --- |
| `picasso-<version>-<commit>-windows-x64.exe` | The app, ready to run |
| `picasso-<version>-<commit>-windows-x64.zip` | The exe, this README, and the license |
| `picasso-<version>-<commit>-linux-x64.tar.gz` | The `picasso` binary, `picasso.desktop`, `picasso.png`, this README, and the license |
| `picasso-<version>-<commit>-linux-x64.deb` | A Debian / Ubuntu package that installs the app, its menu entry, and its icon |
| `picasso-<version>-<commit>-linux-x64.AppImage` | The app with the libraries it needs, for Ubuntu, RHEL, and other distributions, run without installing |
| `picasso-<version>-<commit>-macos-universal.dmg` | A disk image with `Picasso.app`, a shortcut to Applications, this README, and the license |
| `picasso-<version>-<commit>-macos-universal.zip` | `Picasso.app` on its own |

The AppImage needs no installing and no administrator rights, so it suits computers where you can't add system packages. It is built on AlmaLinux 8, so it runs on RHEL 8, Ubuntu 20.04, Debian 10, and newer. It carries every library Picasso uses (ALSA, fontconfig, X11, Wayland, and xkbcommon, with their dependencies) except glibc and the OpenGL driver, which come from the system. On start, it uses the system's copy of each library when there is one and its own copy otherwise. Run it with:

```bash
chmod +x picasso-<version>-linux-x64.AppImage
./picasso-<version>-linux-x64.AppImage
```

AppImages mount themselves with FUSE. Where FUSE is missing, as in containers, run `./picasso-<version>-linux-x64.AppImage --appimage-extract-and-run`, or unpack it once with `--appimage-extract` and run `squashfs-root/AppRun`. FFmpeg is not included. Without permission to install it, download a static build and put `ffmpeg` and `ffprobe` in the AppImage's folder or in an `ffmpeg` folder beside it, and Picasso finds them. CI starts each AppImage on Ubuntu 22.04, Ubuntu 24.04, AlmaLinux 8, and AlmaLinux 9 before publishing it.

The `.deb` and `.tar.gz` are built on Ubuntu 22.04, so they run on that release and newer. Install the `.deb` with `sudo apt install ./picasso-<version>-linux-x64.deb`, which also pulls in the ALSA and OpenGL libraries. To use the `.tar.gz` instead, put `picasso` on your `PATH`, copy `picasso.desktop` to `~/.local/share/applications/`, and copy `picasso.png` to `~/.local/share/icons/hicolor/256x256/apps/`.

The macOS app is a universal binary for Apple silicon and Intel Macs, and runs on macOS 11 and newer. Open the `.dmg` and drag Picasso to Applications. The app has an ad-hoc signature but is not notarized by Apple, so the first launch is blocked: Control-click Picasso and choose **Open**, or click **Open Anyway** in **System Settings → Privacy & Security**. From a terminal, `xattr -dr com.apple.quarantine /Applications/Picasso.app` does the same.

To publish a release, set `version` in `Cargo.toml`, commit, then push a matching tag:

```bash
git tag v0.9.3
git push origin v0.9.3
```

The workflow then attaches all six packages (named `picasso-0.9.3-windows-x64.exe` and so on) and `SHA256SUMS.txt` to a new GitHub release. The run fails if the tag doesn't match the version in `Cargo.toml`. A tag with a hyphen, such as `v1.0.0-beta.1`, is marked as a pre-release. Release builds open no console window.

## Data

The library database (`picasso.db`) lives in the app data directory. Thumbnails (`thumbs.db`) and map tiles live in the cache directory, which is safe to delete. Your choices in **Settings** are kept in `settings.json` in the settings folder, apart from the library.

| OS | Library | Cache | Settings |
| --- | --- | --- | --- |
| Windows | `%APPDATA%\Picasso\picasso\data` | `%LOCALAPPDATA%\Picasso\picasso\cache` | `%APPDATA%\Picasso\picasso\config\settings.json` |
| macOS | `~/Library/Application Support/app.Picasso.picasso` | `~/Library/Caches/app.Picasso.picasso` | `~/Library/Application Support/app.Picasso.picasso/settings.json` |
| Linux | `~/.local/share/picasso` | `~/.cache/picasso` | `~/.config/picasso/settings.json` |

**Settings → Storage** shows all three with their sizes. It opens the library or cache folder, shows `settings.json` in the file manager, and clears the cache. Picasso makes thumbnails again as photos come on screen, and fills in the rest in the background the next time it starts.

`settings.json` is plain JSON, so you can back it up or copy it to another computer. The CARTO API key in it is encrypted for this computer only and has to be entered again on another one.

Picasso backs up the library database once a week, when it starts, into a `backups` folder next to `picasso.db`, and keeps the five newest (`picasso-YYYYMMDD-HHMMSS.db`). **Settings → Storage** makes one on demand and restores one: Picasso backs up the current library first, then restarts with the restored one. The cache also holds `hashes.db`, the file hashes of the last duplicate search, so the next search reads only new or changed files.

## Settings

Click the gear at the top right, or press Ctrl+, (⌘, on macOS), to open **Settings**:

- **General:** interface language, color theme, thumbnail size, slideshow interval, whether to ask before moving photos to the trash, and **Updates**: whether to check once a day for a newer release, with **Check now**. The check only asks GitHub for the latest version number; when there is a newer one, a link to its download page appears at the top right.
- **Folders:** every folder Picasso watches, each with buttons to open it, rescan it, or remove it from the library (the files stay on disk), plus **Import** and **Rescan all folders**. **Excluded folders** lists subfolders Picasso skips, with **Include again**; **Exclude a folder…** adds one, as does **Exclude from scanning** on a subfolder's right-click menu in the folder tree. An excluded folder's photos leave the library, and their stars, captions, and edits come back if you include it again within 90 days.
- **File types:** which files Picasso adds when it scans: JPEG, PNG, GIF, WebP, BMP, TIFF, HEIC/HEIF, camera RAW, and videos, with **Select all**. When the dialog closes, every folder is rescanned. Files of a type you turned off leave the library, with their stars, captions, and edits, but stay on disk.
- **Map:** the map shown in Places and the Map panel:
  - **OpenFreeMap Liberty** (the default) is free and needs no key. Picasso downloads OpenFreeMap's vector data and draws it in the colors of the Liberty style, with place, water, and road names in the app language.
  - **OpenStreetMap** is the standard OpenStreetMap map.
  - **CartoDB Positron** is a light, quiet map. It needs a free CARTO API key, which you paste into the field under it; **Get a free key** opens CARTO's sign-up page. The key is saved encrypted with a key derived from this computer's machine ID, so it shows as `********` afterwards and a copied `settings.json` doesn't work on another computer. **Remove** clears it.
  - **Esri World Imagery** shows satellite and aerial photos.

  Each source keeps its own tiles in the cache.
- **Storage:** where the library database, the cache, and `settings.json` live, how much space each takes, and **Clear cache**. **Show in folder** next to the settings file opens the file manager with `settings.json` selected. **Backups** shows the latest backup with **Back up now**, **Restore…**, and **Open folder**.

Settings apply right away and are saved in `settings.json` in the system's settings folder, so they stay the same whichever library you open:

| OS | Settings file |
| --- | --- |
| Windows | `%APPDATA%\Picasso\picasso\config\settings.json` |
| macOS | `~/Library/Application Support/app.Picasso.picasso/settings.json` |
| Linux | `~/.config/picasso/settings.json` |

The first time a new version starts, it copies the settings an older version kept in the library.

## What it does

- Folder library with a date-grouped thumbnail grid, search, star filter, and sorting by date (newest or oldest first) or by name. **Sort by Name, edited** shows only the photos you have edited. Dragging the scroll bar shows the date in view, or the first file name when sorted by name. When sorted by date, a strip beside the grid lists the years, with the months of the year in view; click one to jump there. The library is read on a background thread, so the window stays responsive while a large library loads or a scan adds photos.
- Search finds photos that match every word you type, in the file name, the folder path below the library folder, the caption, the camera, or an album name. A date such as `2019`, `2019-05`, or `2019-05-17` (also with `/` or `.`) finds photos taken in that year, month, or day, so `2019-05 beach` finds the May 2019 beach photos.
- Folder tree with photo counts, albums, starred photos, and Places, a map of every photo with GPS. The map comes from OpenFreeMap, OpenStreetMap, CARTO, or Esri; pick one in **Settings → Map**.
- GPS tracks: `.gpx` and `.kml` files in your folders are read by the scan and drawn as colored paths under the photo pins, in Places and on the Map panel. GPX tracks and routes are read, as are KML lines and `gx:Track` recordings; KMZ files and the outlines of areas are not. Hover over a track for its name, start time, distance, and duration, and double-click it to zoom to it. **Tracks** in the Places toolbar turns them off. Like photos, a track file is read again only when it changes, and one that can't be read is listed with the other unreadable files.
- Info and Map panel beside the grid with the photo's EXIF and location, and a details panel in the viewer with a histogram
- Viewer with zoom, pan, filmstrip, rotate, slideshow, and video playback. Zoom with the wheel, Ctrl+wheel, a trackpad pinch, or + and −. The viewer first shows a 2560 px version of the photo; zoom past it, or press 1 for actual size, and it decodes the photo at full resolution so 100% shows every pixel. **Show in folder** opens the file manager with the file selected.
- Photo Tray for working on a selection: add to album, rotate, star, delete, or export. After **Move to trash**, the status bar offers **Undo** for two minutes, which puts the files back where they were (Windows and Linux).
- **Export…** in the Photo Tray, or on a photo's right-click menu, saves the selected photos with their edits into one folder as JPEG, PNG, or TIFF, at full size or a long edge of 4096, 2048, 1600, or 1080 px, with a JPEG quality. Each file keeps its name with the new extension and is numbered when the name is taken, so nothing is overwritten. Videos are left out. Three photos are saved at a time in the background; the status bar shows the progress with **Stop**, then **Open folder**. Picasso remembers the folder and choices.
- **Duplicates** in the sidebar finds files whose content is identical. Files with the same size, pixel size, and date taken are compared by their first 64 KB, and those still alike are read in full and compared by SHA-256, in folder order so a hard disk reads on through each folder. The hashes are remembered with each file's size and modification time, so a later search reads only new or changed files. The search runs in the background and goes on when you close the window; the sidebar shows how many groups it found. Each group keeps one file, a starred or edited one when there is one, and offers the others for the trash, one by one, by group, or all at once.
- **Adjust date…** on a photo's right-click menu fixes the date taken of the selected photos, for a camera whose clock was off or a scanned print: shift them by days, hours, and minutes, set the earliest one to a date and move the others with it, or go back to the date in the file. The new date is kept in the library and outlasts rescans; the files aren't changed.
- Files a scan can't read are listed from a link in the status bar, with **Show in folder** and **Move to trash** for each. The list is kept in the library, so it shows right away on the next start, and an unchanged file isn't opened again on later scans for a week.
- Albums, captions, and stars. A photo's stars, caption, rotation, albums, pin, and edits are kept for 90 days after it leaves the library. They come back when the file reappears at the same path, for example after Undo or after restoring it from the trash yourself. They also come back when it turns up under another name or in another library folder with the same size, modification time, and date taken. A folder that can't be read during a scan, such as a network share that drops out, keeps its photos rather than removing them.
- Background scan that picks up new photos, plus a folder watch. Folders are listed in parallel and unchanged files are skipped without being opened; new files are read once from their first 256 KB for size, date, camera, and GPS. Thumbnails for photos on screen are made right away; the rest are made after the scan, so the two don't compete for the disk. JPEGs are decoded at reduced size for thumbnails, and HEIC files use the thumbnail stored inside them.
- Import of stars, captions, and rotation from a folder's `.picasa.ini`
- Non-destructive editor modeled on Lightcraft's Develop module: light, tone curve, color mixer, color grading, detail, crop, masks, presets, and export to a new file
- Light, dark, or system color theme, and ten interface languages
- About dialog with the version and build date: click the logo at the top left

The app icon is `build/picasso.ico`, which holds every size from 16 to 256 px. At build time `build.rs` unpacks the 256 px image for the window and the in-app logo. On Windows it also embeds the icon in `picasso.exe` together with the version details. Without the Windows SDK, the build still succeeds and prints a warning, and the window still shows the icon. To change the icon, replace `build/picasso.ico` with one whose 256 px image is stored as a 32-bit bitmap rather than PNG, then rebuild. The Linux packages use `build/picasso.png`, the same 256 px image, with `build/picasso.desktop`, and the macOS app turns the PNG into `picasso.icns`; update the PNG along with the `.ico`.

## Editing

Open a photo and press E, or click "Edit" in the viewer toolbar. The editor replaces the details panel. At the top are three tools (**Adjust**, **Crop**, **Mask**), a live histogram, and buttons for history, comparing, clipping, Auto, Reset all, and copy/paste.

The **Adjust** tool has these sections:

- **Profile:** Color, Neutral, Vivid, Landscape, Portrait, or Monochrome, as a starting look under the sliders
- **Presets:** 25 built-in looks in six groups (Color, B&W, Film, Landscape, Portrait, Style) with an **Amount** slider to blend them in. Save the current settings as your own preset; user presets are kept in the library.
- **Transform:** turn the photo by any angle from −180° to 180° with the slider, or type an exact one in **Angle**; tilt it with **Perspective** Vertical and Horizontal (±45°, positive widens the top or the right, for buildings that lean back); rotate 90° left or right; and flip horizontally or vertically, without switching to Crop. Past ±45° the photo stands on its side, so at 90° the whole photo shows in portrait. While you drag Straighten or Perspective, a square grid over the photo helps line up horizons and verticals; **Show grid** keeps it on. **Constrain crop** (on by default) crops away the empty edges a turn, tilt, or moved corner leaves, at the chosen **Aspect**; turn it off to keep them. **Corners…** and **Crop…** open the Transform and Crop tools, and **Reset transform** puts all of it back.
- **Light:** exposure, contrast, highlights, shadows, whites, blacks. Highlights and shadows are edge-aware, so they brighten or darken areas without halos.
- **Tone curve:** a parametric curve (shadows, darks, lights, highlights, with movable splits) and a point curve for RGB, red, green, and blue. Click the graph to add a point, drag it, and double-click it to remove it.
- **HDR:** tone mapping that opens shadows and brings back highlights in one photo, with a **Method**, **Strength**, and **Detail**. Every method works on the same edge-aware base layer as Highlights and Shadows and puts the fine detail back on top, so edges get no halos. The methods are:
  - **Local tone mapping**, which compresses brightness by area in the manner of Durand and Dorsey, for the classic HDR look;
  - **Exposure fusion**, which blends five virtual exposures and keeps each area from the best exposed, after Mertens, Kautz, and Van Reeth, for a natural result;
  - **Reinhard**, the photographic operator, which rolls off highlights gently;
  - **Drago**, an adaptive logarithmic curve that opens deep shadows the most;
  - **Filmic**, the ACES film curve, with rich contrast and soft highlights.

  Raise **Exposure** first to pull more out of the shadows; HDR brings back the highlights it pushes past white. The **HDR Natural** and **HDR Dramatic** presets in the Style group are good starting points.
- **Color:** white balance presets (Daylight, Cloudy, Shade, Tungsten, Fluorescent, Flash), Auto, and the ⊙ eyedropper to click something neutral; temperature, tint, vibrance, saturation
- **Color mixer:** hue, saturation, and luminance for eight color bands
- **Color grading:** shadows, midtones, and highlights wheels plus a global wheel, with blending and balance
- **Effects:** texture, clarity, dehaze, vignette, grain
- **Detail:** sharpening and noise reduction
- **Black & White:** converts to monochrome with an eight-band gray mix

**Crop** draws a frame over the photo. Drag the edges or corners to resize, drag inside to move, and choose an aspect ratio (Free, Original, 1:1, 4:5, 5:7, 2:3, 3:4, 9:16, 10:16; ⇄ swaps portrait and landscape). Overlay guides are rule of thirds, golden ratio, grid, or diagonals. Straighten turns the photo by any angle up to ±180° and keeps the crop inside the photo, at the chosen aspect ratio; while you drag it, a square grid replaces the overlay. Flip horizontal or vertical and rotate by 90° are here too. Press Enter, Esc, or "Done" to leave Crop.

**Transform** (T) puts a handle on each of the photo's four corners and on the middle of each side, so one tool turns, tilts in perspective, skews, and stretches. Drag a corner anywhere to move just that corner; drag the middle of a side to slide that side along itself (skew) or across (keystone and stretch); drag inside the outline to shift the whole photo; and drag outside it to turn it about its middle. The grid stays on to line things up against, and the thin frame shows the crop that will be kept. The Straighten and Perspective sliders work here too, and **Reset corners** puts just the corners back.

**Mask** adds linear (graduated) or radial masks. Drag a mask's handles on the photo to place it, then set feather, invert, and its own exposure, contrast, highlights, shadows, temperature, tint, saturation, clarity, and dehaze. A checkbox shows the selected mask's area in red.

Edited photos show a small sliders badge in the top-right corner of their thumbnail in the library and filmstrip. Click the badge in the library to open the photo in the editor.

Editing is non-destructive. Each slider release, toggle, preset, or "Reset all" becomes one step in the photo's history. The steps are saved in the library, and the original file is never written. The preview, the full-size view, and the thumbnail are all rendered from the original plus the applied steps. While a slider is being dragged the preview renders at reduced size, then sharpens when you let go.

- **Undo / Redo** step back or forward one step (Ctrl+Z, and Ctrl+Y or Ctrl+Shift+Z).
- **Undo all** goes back to the original, and **Redo all** applies every step again.
- Click a row in **History** to jump to that point. A new change after an undo replaces the undone steps.
- **Before** (\\) shows the original, **Side by side** (Y) shows both, and **Split** (Shift+Y) puts a draggable divider between them.
- **Clipping** (J) marks blown highlights in red and crushed shadows in blue. The histogram shows how much of the photo is clipped.
- Double-click a slider's name or a color wheel to set it back to zero.
- **Copy** (Ctrl+Shift+C) and **Paste** (Ctrl+Shift+V) copy every setting except crop and rotation. Paste also works in the library, on all selected photos, and both are in the thumbnail right-click menu.
- **Export…** renders the full-resolution photo with the applied steps and saves it as a new JPEG, PNG, or TIFF (`name-edited.jpg` by default). Choose the format, a long-edge size (original, 4096, 2048, 1600, or 1080 px), and the JPEG quality. The original can't be chosen as the target.

The controls follow [Lightcraft](https://github.com/storytold/lightcraft)'s Develop module and run on the CPU in linear light, with color work in OkLab. Lightcraft's AI masks, RAW processing, healing, merging several photos into an HDR or panorama, and lens profiles are not included. Videos can't be edited.

## Language

Picasso is available in English, 繁體中文 (Traditional Chinese), 简体中文 (Simplified Chinese), 日本語 (Japanese), 한국어 (Korean), Deutsch (German), Español (Spanish), Français (French), Italiano (Italian), and Português (Brazilian Portuguese).

On first launch it uses the system language:

- `zh-TW`, `zh-HK`, `zh-MO`, and `zh-Hant` give Traditional Chinese, and any other Chinese locale gives Simplified Chinese.
- Every Portuguese locale, including `pt-PT`, gives Brazilian Portuguese.
- The other languages match on the language alone, so `fr-CA` gives French and `es-MX` gives Spanish.
- Anything else falls back to English.

To choose a different language, open **Settings → General**. "System language" goes back to following the operating system. Dates in the library headings are written the way each language writes them.

Translations live in `src/i18n/`, one file per language, as pairs of English text and its translation. To add a language, copy one of those files and translate the second column. Then add a variant to `Lang` in `src/i18n.rs`, and a date format to `long_date` in `src/types.rs`. The tests fail if a language is missing a string or drops a `{}` placeholder.

## Color theme

Every page uses the same theme: the library, the viewer, the editor, the side panels, and the dialogs. Pick **Light theme**, **Dark theme**, or **System theme** in **Settings → General**. System theme follows the operating system and switches when it does. The viewer toolbar also has a button that flips between light and dark.

## Shortcuts

Press **F1** or **?**, or click the **?** button at the top right of the library or the viewer, to open **Help & Shortcuts**. It lists every keyboard shortcut (General, Library, Viewer, Video, and Editor) next to the mouse and trackpad gestures (Library, Viewer, Map, and Editor), and follows the app language and theme. Press F1 or ? again to close it.

| Key | Action |
| --- | --- |
| Enter | Open the selected photo |
| Esc | Close the editor, go back, or leave slideshow |
| Arrows | Move the selection, or previous / next photo |
| Home / End | First / last photo in the viewer |
| Ctrl+A | Select every photo in the grid |
| Space | Play or pause a video, or start or pause the slideshow |
| M | Mute or unmute a video |
| S | Star |
| R / Shift+R (or Ctrl+] / Ctrl+[) | Rotate clockwise / counter-clockwise |
| I | Details panel |
| E | Edit the photo |
| Ctrl+Z / Ctrl+Y | Undo / redo an edit step |
| \\ | Show the original while editing |
| Y / Shift+Y | Side-by-side / split before and after |
| J | Show clipped highlights and shadows |
| C | Crop tool (Enter to finish) |
| T | Transform tool with four corners (Enter to finish) |
| Ctrl+Shift+C / Ctrl+Shift+V | Copy / paste edit settings |
| + / −, Ctrl+wheel, pinch | Zoom |
| 0 / 1 | Fit / actual size |
| Delete | Move to the system trash |
| F1 / ? | Open or close Help & Shortcuts |
| Ctrl+, | Settings |
| F5 | Rescan |
| F11 | Fullscreen |

## Formats

JPEG, PNG, GIF, WebP, BMP, TIFF, HEIC/HEIF, and camera RAW (DNG, CR2, CR3, NEF, ARW, RAF, ORF, RW2, PEF, SRW, 3FR, IIQ, X3F, and more). RAW files from cameras newer than the RAW decoder, such as the Hasselblad X2D II, show the large JPEG preview the camera stores in the file. Turn types off in **Settings → File types**. Rotation is stored in the library and applied when you view the photo; the original file is not rewritten.

Videos (MP4, MOV, M4V, AVI, MKV, WebM, WMV, MPEG, MTS/M2TS, 3GP, and more) show in the grid with a play badge and their length, and play inside the viewer with sound. Video needs FFmpeg; see [Prerequisites](#prerequisites). Videos found while FFmpeg is missing are indexed again once it is installed.

## License

MIT. See [LICENSE](LICENSE).
