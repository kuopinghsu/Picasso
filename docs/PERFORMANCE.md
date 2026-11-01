# Performance

How Picasso's speed and memory are measured, the numbers to compare against, and what to expect from different kinds of storage.

## Benchmark

```sh
make bench                                                    # 100,000 catalog rows, 2,000 photo files
make bench PICASSO_BENCH_PHOTOS=500000 PICASSO_BENCH_FILES=10000
make bench-check                                              # small run that fails past the limits, as CI runs it
```

The benchmark (`src/bench.rs`) is a test that `cargo test` skips. It runs in five parts:

- **Catalog:** writes `PICASSO_BENCH_PHOTOS` photos into a new library, then times opening it and the queries the window makes: listing the library by date and by name, starred photos, the folder tree, one folder, searches, the list a rescan compares against, and duplicate candidates.
- **Trip:** puts `PICASSO_BENCH_TRIP` photos (default 10,000), taken over a year, in a Trip in a library of their own. It times adding them, keeping their capture times, **Suggest days** making 365 Days, and **Suggest places**. It also times the check for changes the Trips view makes each frame, the queries it makes again once the library has changed (the Days, the photos without a Day, the review counts and a filter's count), listing the Trip and one Day, a Story made from all 365 Days, and the list of Trips. The queries the Trips view repeats are run 20 or 50 times each, and the median and the 95th percentile (p95) are reported, with the limit on the p95. One photo in 50 has no time-zone offset, so the review counts aren't empty.
- **Scrolling:** makes `PICASSO_BENCH_TRIP` small JPEGs (480×320) once in `target/bench-trip-photos-<n>`, puts them in a Trip over 365 Days, and opens the app on that library without a window (`src/app/headless.rs`). In a 1280×800 window, at 60 frames a second, it scrolls the Trip to the bottom in `PICASSO_BENCH_FRAMES` frames (default 600) as a trackpad fling would, then back up for as many frames at one window height a second, slowly enough for the thumbnails to be decoded and shown. Each frame's time is the app's update and turning what it drew into triangles, as the window counts it, without the graphics card's part. The median, the p95 and the frames over 16.7 ms are reported for each direction.
- **Files:** makes `PICASSO_BENCH_FILES` 1600×1200 JPEGs once in `target/bench-photos-<n>`, then scans them with the same indexer the app uses, rescans with nothing changed, makes a thumbnail of each on the app's number of thumbnail threads, and opens one photo for the viewer.
- **Publication:** makes a Story of the first `PICASSO_BENCH_ASSETS` of those photos (default 300) in galleries of 20. It times exporting it as a web page at the default size, then rendering and sending it at **Low** resolution to a stand-in for a WordPress site that answers at once, so the time is Picasso's own, and publishing it again with nothing changed. The photos repeat every 200, and a photo with the same bytes is sent once, so 300 photos make 200 uploads.

It also reports the peak memory of the process after each part (macOS and Linux).

`make bench-check` sets `PICASSO_BENCH_CHECK=1` with 20,000 rows and 500 files. A result past its limit fails the run. The limits are loose, because CI machines are slow and shared: one second for opening the library and for each query, 50 ms for the Trips view's queries after a change and for a frame while scrolling (p95), 5 or 10 seconds for each whole-Trip job, at least 200 photos a second for the scan, 20 for thumbnails, and 5 for exporting and for publishing, 2 seconds to publish again with nothing changed, half a second to open a photo, and 400 MB of memory. They catch large regressions only; compare against the baselines below for anything finer.

### Cold and warm cache

Files read since the computer started stay in the system's file cache, so a second run measures a warm cache. A cold cache is what the first scan after a restart sees. To measure it, empty the cache right before the run:

- macOS: `sudo purge`
- Linux: `sync; echo 3 | sudo tee /proc/sys/vm/drop_caches`
- Windows: restart, or use RAMMap's **Empty → Empty Standby List**.

## Frame times

Start Picasso with `PICASSO_FRAME_STATS=1` to print frame times to the terminal every five seconds of drawing:

```
frames 300 · cpu per frame: median 2.1 ms, 95% 4.0 ms, slowest 9.8 ms · over 16.7 ms: 0
```

A frame over 16.7 ms misses a 60 Hz refresh and shows as a stutter. To check browsing, scroll the Library of a large library quickly from top to bottom, drag the date strip, and open and leave the viewer, while a first scan runs and after it ends. Watch memory and CPU in Activity Monitor, Task Manager, or `top`. macOS doesn't draw a window while the screen is locked, so no lines appear then.

`make bench` measures scrolling a Trip without a window (see Scrolling above). These lines add what only a real window shows: the graphics card's part of each frame, dragging the date strip, the viewer, and frames during a scan.

## Baselines

Apple M2 Max (12 cores, 32 GB), internal SSD, macOS, warm cache, release build of the changes after 0.9.4.

| Catalog | 10,000 | 100,000 | 500,000 |
| --- | ---: | ---: | ---: |
| Open the library (startup) | 2.0 ms | 14.7 ms | 72.6 ms |
| List the library, newest first | 6.5 ms | 63.3 ms | 320 ms |
| List the library by name | 4.9 ms | 51.3 ms | 265 ms |
| List starred photos | 0.9 ms | 17.5 ms | 85.5 ms |
| Folder tree | 1.4 ms | 12.6 ms | 63.1 ms |
| Open one folder of 50 | 0.1 ms | 0.1 ms | 0.1 ms |
| Search one word | 5.3 ms | 57.6 ms | 293 ms |
| Search a month | 0.2 ms | 0.7 ms | 0.6 ms |
| Rescan check of a folder | 3.5 ms | 35.6 ms | 196 ms |
| Duplicate candidates | 2.8 ms | 31.5 ms | 164 ms |
| Library file size | 4.6 MB | 45.6 MB | 229 MB |
| Peak memory | — | 117 MB | 555 MB |

| Files | 2,000 | 10,000 |
| --- | ---: | ---: |
| First scan | 46 ms (43,000 photos/s) | 372 ms (27,000 photos/s) |
| Rescan with nothing changed | 5.5 ms | 27 ms |
| Thumbnails on 8 threads | 2.73 s (730 photos/s) | 13.8 s (720 photos/s) |
| Open one photo for the viewer | 4.0 ms | 3.5 ms |
| Peak memory after thumbnails | 126 MB | 622 MB (with 500,000 rows) |

| Trip of 10,000 photos over 365 Days | after 0.9.6 (median, p95) |
| --- | ---: |
| Add the photos to the Trip | 109 ms |
| Keep their capture times | 113 ms |
| Suggest days (makes 365 Days) | 44.8 ms |
| The Trips view's check for changes, each frame | 0.9 µs |
| The Trips view's queries after a change | 3.0 ms, p95 3.8 ms |
| List the Trip's photos | 12.8 ms, p95 14.4 ms |
| Open one Day | 0.0 ms, p95 0.1 ms |
| Starred photos of the Trip | 2.1 ms, p95 2.5 ms |
| Suggest places | 29.8 ms |
| A Story from all 365 Days | 53.6 ms |
| List Trips | 0.9 ms, p95 1.2 ms |

The Trip's numbers are the same with 20,000 or 100,000 other photos in the catalog, and peak memory doesn't grow past the catalog's. Keeping capture times measures the library's side only; reading them from the files is limited by the disk, like the scan. The Trips view keeps what it read until the library changes: a write of its own, or a commit by the scanner. Between changes, a frame costs only the 0.9 µs check, and the 3.2 ms of queries come once after each change.

| Scrolling a Trip of 10,000 photos over 365 Days | after 0.9.6 (median, p95) |
| --- | ---: |
| A frame, flung to the bottom in 10 seconds | 1.9 ms, p95 2.1 ms |
| A frame, back up at one window height a second | 0.4 ms, p95 1.1 ms |
| Frames over 16.7 ms | 0 of 600 each way |
| Peak memory after scrolling | 232 MB |

The first run of this part found that a fling queued the thumbnails decoded for photos already scrolled past: the window takes in 16 results a frame, and the decoders finished about 18. After 600 frames, 1,200 thumbnails were waiting and the process held 1.36 GB. The window now drops a decoded thumbnail of a photo it no longer shows as soon as it reads it, without uploading it or counting it in the 16; at most about 40 wait, and the photo is asked for again if it comes back into view. The thumbnail it would have replaced is dropped with it, since a scan sends one when a file has changed, and the old one would otherwise show until it left the cache. `make bench-check` fails if memory goes past 400 MB again.

| Publication of 300 photos (1600×1200) | after 0.9.6 |
| --- | ---: |
| Export as a web page at 1600 px | 2.31 s (130 photos/s) |
| Render and send at Low resolution | 4.42 s (68 photos/s) |
| Sent to the site (200 uploads) | 13.0 MB |
| Publish it again with nothing changed | 2.1 ms |
| Peak memory after publishing | 252 MB |

Both render three photos at a time, as exporting photos from the Library does. Each holds its full-size image while it is made, so memory grows with the photos' size, not their number. With 91 camera photos of 3840×2560, one at a time exported 5 photos a second and peaked at 494 MB; three at a time export 16 a second and peak at 735 MB. A photo rendered earlier in the session to bytes the site has isn't rendered again, so publishing the Story again with nothing changed takes milliseconds, its time going to the site's answers. A real site adds its own time for each upload and request on top.

The scan reads only the start of each file for its size, date, and camera, so it is limited by the disk far more than by the processor: expect it to be many times slower on a cold cache or a hard disk. Thumbnails are made after the scan, newest first, and the ones on screen go first.

The first baseline found that making thumbnails held on to about 1 MB per photo: 4.7 GB after 2,000 photos. The JPEG decoder started a thread per color channel for every photo; it now shares one pool, and the same run peaks at 126 MB. `make bench-check` fails if that comes back.

## Budgets

Each limit of `make bench-check` was set after measuring, far enough above the measurement for a slow CI machine that only a large regression fails it. The measurements are from the baselines above.

| What | Measured | Limit |
| --- | ---: | ---: |
| The Trips view's queries after a change, 10,000 photos | p95 3.8 ms | p95 50 ms |
| List the Trip's 10,000 photos | p95 14.4 ms | p95 1 s |
| Open one Day | p95 0.1 ms | p95 100 ms |
| List Trips | p95 1.2 ms | p95 100 ms |
| Suggest days, 365 Days | 44.8 ms | 10 s |
| Library queries, 100,000 photos | 0.1–68.9 ms | 1 s each |
| First scan | 43,000 photos/s | 200 photos/s |
| Thumbnails | 730 photos/s | 20 photos/s |
| Export a Story | 130 photos/s | 5 photos/s |
| Render and send at Low resolution | 68 photos/s | 5 photos/s |
| Publish again with nothing changed | 2.1–8.4 ms | 2 s |
| Peak memory, each part | 120–254 MB | 400 MB |
| A frame while scrolling a Trip of 10,000 photos | p95 2.1 ms | p95 50 ms |
| A frame of the window, measured by hand | see Frame times | 16.7 ms, a 60 Hz refresh |

The Trips view's check for changes costs 0.9 µs a frame and has no limit of its own. The scrolling limit is three refreshes, loose for a shared CI machine; a frame on the measuring machine stays far below one refresh.

Add a row for each release that changes scanning, the catalog, Trips, thumbnails, exporting, or publishing, and note the machine when it differs.

## The window's thread

The window draws on one thread, so anything slow there shows as a stutter. Each frame it draws from what it already has. The Trips view and the Story editor keep their library reads until the library changes, checked in about a microsecond. Small reads and file checks run there only once for a click, such as moving photos to a Day or checking a folder before it is added. Everything else has a thread of its own, and is bounded:

- **The photo list** is read by a loader thread with its own connection; only the newest request is answered. The list is read whole, not in pages: 69 ms for 100,000 photos and 320 ms for 500,000 (see the baselines). That time is spent off the window's thread, the window keeps showing the earlier list until it arrives, and the grid draws only the rows on screen, so a page at a time would not draw any faster. Sorting, selecting a range and jumping to a date also need the whole list.
- **Thumbnails** are decoded on 2 to 8 threads, three quarters of the cores. A photo is asked for once, and a request for one scrolled out of view is dropped before it is decoded; one decoded after it left the view is dropped before it is uploaded, so a fast scroll doesn't queue images. At most 400 thumbnail textures are kept, and 48 sharper copies for big tiles, or twice what one frame shows if that is more. The least recently drawn go first.
- **The viewer's photo** is decoded on those threads; a newer request for the same photo makes an older one stale, and it is dropped. A file that never answers, as on a share that hangs, holds up only the thread reading it: a test opens one in the viewer and the window keeps drawing and opens the photo beside it.
- **Exporting and publishing** render three photos at a time and send one upload at a time. Their files are written under a temporary name and cleared when the job ends, and on the next run when a crash cut it short.
- **Network replies** are read up to a size: 4 MB from a WordPress site, 16 MB for a map tile, and 1 MB for place names and the update check. Every request runs off the window's thread.
- **Rename** reads the folder, its photos in the library and the renamings it lists on a thread with its own connection, when the dialog opens and again after a scan or a renaming. Rename and Undo stay off until it is done.
- **Track files** for a route's elevation chart, the library's size in Settings, duplicate checks, Trip packages, and capture times for Suggest days are read on threads of their own.

## Storage

- **Internal SSD:** what the baselines measure.
- **External drives (USB, Thunderbolt):** supported. A drive that is unplugged keeps its photos, stars, and edits in the library; its folder is scanned again when it is back, at the next start or with **F5**. Moving a photo to the trash on a disconnected drive reports it rather than removing it from the library. Hard disks are much slower for a first scan and for thumbnails, which read every file in full.
- **Network drives (SMB, NFS, NAS):** supported for reading, with these limits:
  - Folders are listed with a few threads at once to hide network latency, but a first scan and the thumbnails still read every photo over the network.
  - Changes made from another computer may not be noticed while Picasso runs, because many network file systems don't report them. Press **F5** to rescan.
  - A share that drops out during a scan keeps its photos rather than removing them.
  - The library itself (`picasso.db`) must stay on a local disk. SQLite, which keeps it, can't lock files reliably over a network.
- **Cloud folders (iCloud Drive, OneDrive, Dropbox):** files kept only online are downloaded when Picasso reads them for the scan or a thumbnail. Keep the folder downloaded, or expect the first scan to download it.
