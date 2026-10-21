# Picasso

A fast local photo library and viewer, in the spirit of Picasa. It watches folders, builds a thumbnail grid, and finds faces on this computer. Nothing is uploaded.

## Run

```bash
cargo run --release
cargo run --release -- ~/Pictures ~/Photos
```

The first launch also adds your Pictures folder when it exists. Later launches rescan every folder you have added. Debug builds are usable, but face detection is much faster with `--release`.

Picasso draws with OpenGL. On WSL that uses the desktop GPU. Set `PICASSO_RENDERER=wgpu` to use Vulkan instead.

Library data lives under the app data directory (`~/.local/share/picasso` on Linux). Thumbnails and face crops live in the cache directory (`~/.cache/picasso`).

## What it does

- Folder library with a date-grouped thumbnail grid, search, and star filter
- Lightbox with zoom, pan, filmstrip, rotate, slideshow, and EXIF details
- Albums, captions, and stars
- Background scan that picks up new photos, plus a folder watch
- Face detection on each new photo, drawn as boxes in the viewer
- People groups. Name a face and Picasso looks for more that look like that person
- Import of stars, captions, rotation, and face rectangles from a folder's `.picasa.ini`

## Shortcuts

| Key | Action |
| --- | --- |
| Enter | Open the selected photo |
| Esc | Back, or leave slideshow |
| Arrows | Move the selection, or previous / next photo |
| Space | Start or pause the slideshow |
| S | Star |
| R / Shift+R | Rotate clockwise / counter-clockwise |
| F | Show or hide face boxes |
| I | Details panel |
| + / − | Zoom |
| 0 / 1 | Fit / actual size |
| Delete | Move to the system trash |
| F5 | Rescan |
| F11 | Fullscreen |

Click a face box to name the person. "Not this person" splits that face into its own group.

## Faces

Detection uses the bundled SeetaFace frontal model (BSD-2-Clause, from the [rustface](https://github.com/atomashpolskiy/rustface) port). Grouping compares a small local descriptor, so it works best on clear frontal faces and can be corrected by naming or splitting. Photos are never sent off the machine.

## Formats

JPEG, PNG, GIF, WebP, BMP, and TIFF. Rotation is stored in the library and applied when you view the photo; the original file is not rewritten.
