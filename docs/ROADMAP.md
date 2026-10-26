# Roadmap

Ideas that were weighed against what Picasso already does, with where each stands. None is promised: what gets built next is chosen from what users ask for in the issues, and from what it costs in code and in a simpler window. Picasso aims to stay a quick viewer and organizer with a capable editor, not a catalog manager with a panel for everything.

## Searching with more than words

**Today:** search matches every word typed against the file name, the folder path, the caption, the camera, the lens, and album names, and a date such as `2019-05` finds the photos of that month. The star filter, **Sort by Name, edited**, and albums narrow the grid in other ways; Places finds photos on a map, and **Duplicates** lists files with the same content. In **Statistics**, clicking a day, month, camera, or lens shows those photos, and clicking a year narrows the statistics to it.

**Not yet:** focal length, ISO, aperture, shutter speed, file format, pixel size, having GPS, and having edits can't be searched, and words can't be limited to one field. The library keeps the camera, lens, focal length, pixel size, and GPS position of each photo; ISO, aperture, and shutter speed are read from the file when the Info panel shows them.

**Idea:** words like `iso:3200`, `lens:35mm`, `f:1.8`, `type:heic`, `gps:no`, or `edited` in the same search box, so that nothing new is added to the window. ISO, aperture, and shutter speed would need to be kept when a photo is scanned, which means a new library version (see [`DATABASE.md`](DATABASE.md)) and reading every photo once more after the upgrade. Focal length, format, pixel size, GPS, and edits could come first, since the library already knows them.

## Saved searches

**Today:** albums keep a hand-picked set of photos.

**Idea:** a search can be kept as an album that fills itself. It is worth doing only together with the search words above, since a search of a few words is quick to type again. It should look like an album in the sidebar rather than add a new kind of panel.

## Working on many photos at once

**Today:** with photos selected, the toolbar above the grid stars, rotates, and adds them to an album, the Photo Tray deletes and exports them, and **Paste settings** (Ctrl+Shift+V) applies copied edit settings to all of them; the right-click menu has most of these too. **Adjust date…** shifts the dates of a selection. **Export…** saves edited copies of a selection at one size, format, and quality.

**Not yet:** renaming files. Renaming changes the user's files themselves, which Picasso otherwise does only when moving them to the trash; it would need a preview of every new name, a check that no name is taken, and Undo, as the trash has on Windows and Linux. Other export choices, such as a name pattern or keeping the folder layout, are smaller and can follow requests.

## Before and after

Done: `\` shows the original while editing, **Side by side** (Y) shows both, and **Split** (Shift+Y) puts a draggable divider between them.

## Taking the library elsewhere

**Today:** the library database holds everything Picasso adds to the photos, and its backups can be copied to another computer and restored from **Settings → Storage**. Photos that move are recognized and keep their data (see "Data" in the README). Places set from a GPS track are also written beside the photos, and stars, captions, and rotations are read from Picasa's `.picasa.ini`.

**Not yet:** writing stars, captions, and places in a form other programs read, such as XMP sidecar files, and reading them back. That would let the data outlast Picasso, but writing files beside every photo is a change users should choose, so it would be an export on request rather than automatic. The library keeps full paths, so on a computer where the photos are in other places, a restored library needs its folders moved as the README describes: remove the old places and add the new ones, and the data comes back by matching each photo.

## Maps, places, and tracks

**Today:** Places and the Map panel share the map providers, the zoom control, and the scale bar. Places shows clusters of photos and GPS tracks. The Travel Map is drawn offline from built-in country borders; clicking a colored place opens a card whose **Show photos** shows them in Places.

**To review:** whether the search and star filter of the grid should also narrow Places, and whether a track can show the photos taken along it. Both need a look at how people use Places first.

## Keyboard and accessibility

**Today:** the grid, viewer, slideshow, and editor work from the keyboard (see "Shortcuts" in the README and **Help & Shortcuts**, F1). Esc closes the editor and goes back, and most buttons that show only an icon have a tooltip. The window is drawn by egui, which describes its controls to screen readers through AccessKit.

**To check:** Tab order through the sidebar, the Photo Tray, and Settings; that the focused control is always visible in both themes; and what a screen reader says for the grid, whose thumbnails are drawn rather than made of separate buttons. That needs testing with VoiceOver, Narrator, and Orca; [`TESTING.md`](TESTING.md) lists it among the checks by hand before a release.

## Needs feedback from users

These wait for requests in the issues before any work starts: the search words, saved searches, renaming, XMP files, and changes to Places. The checks in [`TESTING.md`](TESTING.md) and the limits in [`PERFORMANCE.md`](PERFORMANCE.md) stay in place whatever is chosen.
