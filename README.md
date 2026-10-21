# Picasso

![Picasso](images/banner.jpg)

A fast local photo library, viewer, and editor in the spirit of Picasa. It watches your folders, builds a date-grouped thumbnail grid, and edits photos without touching the originals. Everything runs on this computer, and nothing is uploaded.

![The Picasso library: folder tree on the left, thumbnail grid in the middle, and photo details on the right](images/picasso.png)

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

GitHub Actions (`.github/workflows/ci.yml`) builds and tests Picasso on Windows for every push to `main` and every pull request. Each run keeps `picasso-<version>-<commit>-windows-x64.exe` and a `.zip` with the exe, this README, and the license as build artifacts.

To publish a release, set `version` in `Cargo.toml`, commit, then push a matching tag:

```bash
git tag v0.9.0
git push origin v0.9.0
```

The workflow then attaches `picasso-0.9.0-windows-x64.exe`, `picasso-0.9.0-windows-x64.zip`, and `SHA256SUMS.txt` to a new GitHub release. The run fails if the tag doesn't match the version in `Cargo.toml`. A tag with a hyphen, such as `v1.0.0-beta.1`, is marked as a pre-release. Release builds open no console window.

## Data

The library database (`picasso.db`) lives in the app data directory. Thumbnails and map tiles live in the cache directory, which is safe to delete.

| OS | Library | Cache |
| --- | --- | --- |
| Windows | `%APPDATA%\Picasso\picasso\data` | `%LOCALAPPDATA%\Picasso\picasso\cache` |
| macOS | `~/Library/Application Support/app.Picasso.picasso` | `~/Library/Caches/app.Picasso.picasso` |
| Linux | `~/.local/share/picasso` | `~/.cache/picasso` |

## What it does

- Folder library with a date-grouped thumbnail grid, search, star filter, and sorting
- Folder tree with photo counts, albums, starred photos, and Places, a map of every photo with GPS
- Info and Map panel beside the grid with the photo's EXIF and location, and a details panel in the viewer with a histogram
- Viewer with zoom, pan, filmstrip, rotate, slideshow, and video playback
- Photo Tray for working on a selection: add to album, rotate, star, or delete
- Albums, captions, and stars
- Background scan that picks up new photos, plus a folder watch. Folders are listed in parallel and unchanged files are skipped without being opened; new files are read once from their first 256 KB for size, date, camera, and GPS. Thumbnails for photos on screen are made right away; the rest are made after the scan, so the two don't compete for the disk. JPEGs are decoded at reduced size for thumbnails, and HEIC files use the thumbnail stored inside them.
- Import of stars, captions, and rotation from a folder's `.picasa.ini`
- Non-destructive editor modeled on Lightcraft's Develop module: light, tone curve, color mixer, color grading, detail, crop, masks, presets, and export to a new file
- Light, dark, or system color theme, and ten interface languages
- About dialog with the version and build date: click the logo at the top left

The app icon is `build/picasso.ico`, which holds every size from 16 to 256 px. At build time `build.rs` unpacks the 256 px image for the window and the in-app logo. On Windows it also embeds the icon in `picasso.exe` together with the version details. Without the Windows SDK, the build still succeeds and prints a warning, and the window still shows the icon. To change the icon, replace `build/picasso.ico` with one whose 256 px image is stored as a 32-bit bitmap rather than PNG, then rebuild.

## Editing

Open a photo and press E, or click "Edit" in the viewer toolbar. The editor replaces the details panel. At the top are three tools (**Adjust**, **Crop**, **Mask**), a live histogram, and buttons for history, comparing, clipping, Auto, Reset all, and copy/paste.

The **Adjust** tool has these sections:

- **Profile:** Color, Neutral, Vivid, Landscape, Portrait, or Monochrome, as a starting look under the sliders
- **Presets:** 23 built-in looks in six groups (Color, B&W, Film, Landscape, Portrait, Style) with an **Amount** slider to blend them in. Save the current settings as your own preset; user presets are kept in the library.
- **Light:** exposure, contrast, highlights, shadows, whites, blacks. Highlights and shadows are edge-aware, so they brighten or darken areas without halos.
- **Tone curve:** a parametric curve (shadows, darks, lights, highlights, with movable splits) and a point curve for RGB, red, green, and blue. Click the graph to add a point, drag it, and double-click it to remove it.
- **Color:** white balance presets (Daylight, Cloudy, Shade, Tungsten, Fluorescent, Flash), Auto, and the ⊙ eyedropper to click something neutral; temperature, tint, vibrance, saturation
- **Color mixer:** hue, saturation, and luminance for eight color bands
- **Color grading:** shadows, midtones, and highlights wheels plus a global wheel, with blending and balance
- **Effects:** texture, clarity, dehaze, vignette, grain
- **Detail:** sharpening and noise reduction
- **Black & White:** converts to monochrome with an eight-band gray mix

**Crop** draws a frame over the photo. Drag the edges or corners to resize, drag inside to move, and choose an aspect ratio (Free, Original, 1:1, 4:5, 5:7, 2:3, 3:4, 9:16, 10:16; ⇄ swaps portrait and landscape). Overlay guides are rule of thirds, golden ratio, grid, or diagonals. Straighten rotates up to ±45° and keeps the crop inside the photo. Flip horizontal or vertical and rotate by 90° are here too. Press Enter, Esc, or "Done" to leave Crop.

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

The controls follow [Lightcraft](https://github.com/storytold/lightcraft)'s Develop module and run on the CPU in linear light, with color work in OkLab. Lightcraft's AI masks, RAW processing, healing, HDR and panorama merge, and lens profiles are not included. Videos can't be edited.

## Language

Picasso is available in English, 繁體中文 (Traditional Chinese), 简体中文 (Simplified Chinese), 日本語 (Japanese), 한국어 (Korean), Deutsch (German), Español (Spanish), Français (French), Italiano (Italian), and Português (Brazilian Portuguese).

On first launch it uses the system language:

- `zh-TW`, `zh-HK`, `zh-MO`, and `zh-Hant` give Traditional Chinese, and any other Chinese locale gives Simplified Chinese.
- Every Portuguese locale, including `pt-PT`, gives Brazilian Portuguese.
- The other languages match on the language alone, so `fr-CA` gives French and `es-MX` gives Spanish.
- Anything else falls back to English.

To choose a different language, use the language button at the top right. "System language" goes back to following the operating system. The choice is saved in the library. Dates in the library headings are written the way each language writes them.

Translations live in `src/i18n/`, one file per language, as pairs of English text and its translation. To add a language, copy one of those files and translate the second column. Then add a variant to `Lang` in `src/i18n.rs`, and a date format to `long_date` in `src/types.rs`. The tests fail if a language is missing a string or drops a `{}` placeholder.

## Color theme

Every page uses the same theme: the library, the viewer, the editor, the side panels, and the dialogs. Use the sun or moon button at the top right to pick **Light theme**, **Dark theme**, or **System theme**. System theme follows the operating system and switches when it does. The viewer toolbar also has a button that flips between light and dark. The choice is saved in the library.

## Shortcuts

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
| Ctrl+Shift+C / Ctrl+Shift+V | Copy / paste edit settings |
| + / − | Zoom |
| 0 / 1 | Fit / actual size |
| Delete | Move to the system trash |
| F1 | Keyboard shortcuts |
| F5 | Rescan |
| F11 | Fullscreen |

## Formats

JPEG, PNG, GIF, WebP, BMP, TIFF, HEIC/HEIF, and camera RAW (DNG, CR2, CR3, NEF, ARW, RAF, ORF, RW2, PEF, SRW, 3FR, IIQ, X3F, and more). Rotation is stored in the library and applied when you view the photo; the original file is not rewritten.

Videos (MP4, MOV, M4V, AVI, MKV, WebM, WMV, MPEG, MTS/M2TS, 3GP, and more) show in the grid with a play badge and their length, and play inside the viewer with sound. Video needs FFmpeg; see [Prerequisites](#prerequisites). Videos found while FFmpeg is missing are indexed again once it is installed.

## License

Picasso is released under the [MIT License](LICENSE).
