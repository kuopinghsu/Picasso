# Trips and Stories

How Picasso organizes travel photos into Trips and writes them up as Stories: the model, who owns what, how photos keep their place in a Trip when files move or leave the library, and how capture times become days. The data layer is in `src/trip.rs` and `src/db/trips.rs`; the tables are listed in [DATABASE.md](DATABASE.md).

This page records decisions. Each one is changed only on purpose, together with its tests.

## Scope

Trips, Days, Stories, preview, and HTML and Markdown export work entirely on this computer: they need no website, AI service, or account. Publishing a Story to a self-hosted WordPress site is optional and builds on the same Story and rendering; it never replaces export.

Booking, payments, social features, hosting, a bundled web server, background sync, and automatic publication are out of scope. No WordPress theme is required: export and publishing work with any theme.

The interface shows only what works. A table may exist before the page that uses it, but no button, menu item, or setting appears for something that doesn't work yet.

## Concepts and ownership

| Thing | Belongs to | Notes |
| --- | --- | --- |
| **Album** | the library | Unchanged. Trips don't use or change Albums. |
| **Trip** | the library | Title, description, destination text, optional date range, a timezone, an optional cover, and a status (active or archived). The date range describes the Trip; it never hides photos outside it. |
| **Day** | one Trip | An ordered section of a Trip, with an optional calendar date, a title, and notes. Days may be empty, and several Days may share a date. |
| **Trip member** | one Trip | A photo or video in the Trip, with its order and, optionally, one Day of the same Trip. A photo can be in several Trips, but in at most one Day of each. Having no Day is valid. |
| **Story** | one Trip | A narrative: title, subtitle, author and date display, cover, template, and an ordered list of blocks. A blank Story has no blocks but still has its Trip. |
| **Story block** | one Story | Heading, paragraph, photo, gallery, separator, or credits, with a stable id and a versioned payload. |
| **Story media** | one block | A photo used in a block, with its own caption and alt text. The same photo can be in several Stories, each with its own words. |
| **Track link** | one Trip, optionally one Day | A link to a GPS track already in the library. The track's points are not copied. |

Rules that follow:

- Trips, Days, and Stories never change a photo's stars, caption, date, edits, place, Albums, or file. Their text is separate from catalog captions unless the user explicitly copies it over.
- New Story photos are picked from the Story's Trip. Adding a photo from outside the Trip adds it to the Trip as well, in the same action.
- Removing a photo from a Trip leaves the Story blocks that use it in place; they show it as removed from the Trip and can be repaired.
- Selection for a Story belongs to that Story. There is no shared "selected for story" flag on a Trip member.
- A Day of one Trip can never be given to a member of another Trip. The database enforces this, not only the interface.
- A suggested Day assignment and a user's assignment are stored differently. Suggesting again replaces only suggestions, never the user's Days, names, notes, order, or Story content.

## Removing things

- **Archive** is the normal way to put a Trip away. An archived Trip keeps everything, including its Stories, and is found through an Archived filter.
- **Delete** a Trip only after a summary of what goes with it: its Days, members, Stories, and track links. Deleting never touches photo files, catalog data, or anything published on a website. Records of what was published stay as detached records, so a later publish can't post a duplicate.
- Deleting a Story removes its blocks; the photos stay in the Trip.
- Removing a photo from the library, or its file disappearing, never deletes Trip or Story content (see below).

## Media identity

A catalog row is not a durable identity: when its file leaves the library the row is deleted, and when the file comes back it gets a new row and id. Catalog ids are never reused (the `id_floor` table), but that only stops an old id from pointing at a different photo; it doesn't bring the old link back. Trips and Stories therefore refer to photos through **media references**:

- Each reference has its own id, which is never reused, and a link to the current catalog row that becomes empty when the row is deleted.
- It keeps repair hints: the path, the file name, the size, the modification time, and the date taken read from the file.
- When the link is empty, the reference is shown as unavailable. The Trip member and the Story block stay.
- After each scan, unlinked references are linked again: first to a photo at the same path, else to the one photo with the same size, modification time, and file date. If more than one photo matches, nothing is linked and the reference waits for the user to choose. A photo that already has a reference is never given a second one.
- References are kept as long as any Trip member, Story, cover, or publication uses them, however long the photo has been gone. The 90-day limit of the photo archive does not apply to them.
- References no longer used by anything are removed.

## Capture time and Days

The catalog's date taken is a display value. For a photo it is the camera's clock reading interpreted in the timezone the computer had when it was scanned, so on its own it does not say when the photo was taken in UTC, nor what the camera's offset was. Days are therefore worked out from separate provenance, and the catalog's dates are never changed for them.

Each Trip has an explicit timezone, chosen or confirmed when it is created: either a named zone (such as `Asia/Taipei`), using timezone data packaged with Picasso so it works offline, or a fixed offset (such as `+08:00`), shown as such and never adjusted for daylight saving. Days are calendar days in that timezone.

A member's instant is decided in this order:

1. An instant the user confirmed for this Trip member.
2. The file's clock reading with the offset the file records (`OffsetTimeOriginal`), plus any **Adjust date** shift the user made in the catalog. The shift is applied once, as the difference between the catalog date and the file's own date.
3. The clock reading without a recorded offset, read as Trip time once the user accepts, in the Trip's settings, that cameras without an offset were set to it. Until then the member stays without a suggested Day and is counted for review.
4. Videos, which record UTC.
5. No clock reading (the file's modification time stood in): **Needs review**.

**Suggest days** reads the clock reading and offset only from Trip members that lack them or whose file changed, on a background thread, never by rescanning the library, and never writes the files. It makes a Day for each date that has none, after the existing Days and in date order, and reports how many photos need review and why. The time zone rules come from `chrono-tz`, compiled into Picasso. A clock reading that is ambiguous or doesn't exist in the Trip timezone (around a daylight saving change) goes to **Needs review**. With the same confirmed inputs, the same Days come out whatever timezone the computer is in.

Members Suggest days couldn't place are kept in **Needs review** with the reason, shown among the Days. Putting them in a Day, or in no Day, settles them; Undo brings the review back. Within a Day, photos keep the order the user gives them, whatever the grid's sort.

GPS tracks already in the library are linked to the Trip from its header, to the whole Trip or to one Day. The link keeps the track's path and name, not its points; while the file is gone it shows as unavailable and comes back when the library finds it.

## Rendering, export, and publishing

- A Story follows the catalog's current edits by default, and shows when its output would change because of them.
- Each export or publish job pins the Story revision, each photo's edit and rotation settings, the output profile, and the identity of each source file. It renders its own images before writing or sending anything, so edits made while it runs affect the next job. If a source file is missing or has changed, the job stops; it never substitutes other content.
- Output images are always rendered from the editing pipeline. Camera originals and RAW files are never copied or uploaded.
- Precise location and other EXIF data are removed by default; any location shown has to be approved first.
- Publishing keeps four things apart: the local content revision, the status last seen on the website, the sync state, and the state of the running job. A failed upload never erases the fact that an earlier version is published. **Save as new draft** makes a new explicit edition; retry and update reuse the existing edition and post.

## Module boundaries

- `src/trip.rs`: Trip, Day, member, Story, and block types, and their validation.
- `src/db/trips.rs`: the tables, and every change to them, each in one transaction. The interface calls these operations with explicit ids and selection order; validation never lives in view code.
- The Trips page, Timeline, Story editor, and exporter live in their own modules under `src/app/` and reuse the library's selection, thumbnails, viewer, editor, and background workers.

## Desktop interface

New pages extend the existing interface rather than adding a visual style of their own:

- Colors, text styles, spacing, corner radii, selection, and hover come from `src/app/theme.rs` (for example `accent()`, `text()`, `muted()`, `select()`, `primary_button`, `danger_button`). Light, dark, and System themes, and the teal accent, work as everywhere else. A missing pattern is added to the theme, not drawn locally.
- Trips is a sidebar destination like Albums and Places. The toolbar, panels, shortcuts, and selection behave as in the Library.
- Each task has one primary action. Secondary actions are grouped, advanced settings are revealed progressively, and nothing required is shown only on hover.
- Every state is visible: default, hover, focus, selected, disabled, loading, error, and success, with keyboard focus and a cue that isn't only color.
- Panels adapt to the smallest window and to high DPI; long translations wrap rather than hide actions.
- Desktop controls are native egui. Only the Story's browser preview and exported pages use web layout and typography of their own.
