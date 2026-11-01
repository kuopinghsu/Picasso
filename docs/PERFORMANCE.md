# Performance

How Picasso's speed and memory are measured, the numbers to compare against, and what to expect from different kinds of storage.

## Benchmark

```sh
make bench                                                    # 100,000 catalog rows, 2,000 photo files
make bench PICASSO_BENCH_PHOTOS=500000 PICASSO_BENCH_FILES=10000
make bench-check                                              # small run that fails past the limits, as CI runs it
```

The benchmark (`src/bench.rs`) is a test that `cargo test` skips. It runs in four parts:

- **Catalog:** writes `PICASSO_BENCH_PHOTOS` photos into a new library, then times opening it and the queries the window makes: listing the library by date and by name, starred photos, the folder tree, one folder, searches, the list a rescan compares against, and duplicate candidates.
- **Trip:** puts `PICASSO_BENCH_TRIP` photos (default 10,000), taken over a year, in a Trip in a library of their own. It times adding them, keeping their capture times, **Suggest days** making 365 Days, and **Suggest places**. It also times the check for changes the Trips view makes each frame, the queries it makes again once the library has changed (the Days, the photos without a Day, the review counts and a filter's count), listing the Trip and one Day, a Story made from all 365 Days, and the list of Trips. One photo in 50 has no time-zone offset, so the review counts aren't empty.
- **Files:** makes `PICASSO_BENCH_FILES` 1600×1200 JPEGs once in `target/bench-photos-<n>`, then scans them with the same indexer the app uses, rescans with nothing changed, makes a thumbnail of each on the app's number of thumbnail threads, and opens one photo for the viewer.
- **Publication:** makes a Story of the first `PICASSO_BENCH_ASSETS` of those photos (default 300) in galleries of 20. It times exporting it as a web page at the default size, then rendering and sending it at **Low** resolution to a stand-in for a WordPress site that answers at once, so the time is Picasso's own, and publishing it again with nothing changed. The photos repeat every 200, and a photo with the same bytes is sent once, so 300 photos make 200 uploads.

It also reports the peak memory of the process after each part (macOS and Linux).

`make bench-check` sets `PICASSO_BENCH_CHECK=1` with 20,000 rows and 500 files. A result past its limit fails the run. The limits are loose, because CI machines are slow and shared: one second for opening the library and for each query, 50 ms for the Trips view's queries after a change, 5 or 10 seconds for each whole-Trip job, at least 200 photos a second for the scan, 20 for thumbnails, and 5 for exporting and for publishing, 2 seconds to publish again with nothing changed, half a second to open a photo, and 400 MB of memory. They catch large regressions only; compare against the baselines below for anything finer.

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

| Trip of 10,000 photos over 365 Days | after 0.9.6 |
| --- | ---: |
| Add the photos to the Trip | 109 ms |
| Keep their capture times | 113 ms |
| Suggest days (makes 365 Days) | 105 ms |
| The Trips view's check for changes, each frame | 0.9 µs |
| The Trips view's queries after a change | 3.2 ms |
| List the Trip's photos | 12.9 ms |
| Open one Day | 0.1 ms |
| Starred photos of the Trip | 2.2 ms |
| Suggest places | 29.8 ms |
| A Story from all 365 Days | 53.6 ms |
| List Trips | 1.0 ms |

The Trip's numbers are the same with 20,000 or 100,000 other photos in the catalog, and peak memory doesn't grow past the catalog's. Keeping capture times measures the library's side only; reading them from the files is limited by the disk, like the scan. The Trips view keeps what it read until the library changes: a write of its own, or a commit by the scanner. Between changes, a frame costs only the 0.9 µs check, and the 3.2 ms of queries come once after each change.

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

Add a row for each release that changes scanning, the catalog, Trips, thumbnails, exporting, or publishing, and note the machine when it differs.

## Storage

- **Internal SSD:** what the baselines measure.
- **External drives (USB, Thunderbolt):** supported. A drive that is unplugged keeps its photos, stars, and edits in the library; its folder is scanned again when it is back, at the next start or with **F5**. Moving a photo to the trash on a disconnected drive reports it rather than removing it from the library. Hard disks are much slower for a first scan and for thumbnails, which read every file in full.
- **Network drives (SMB, NFS, NAS):** supported for reading, with these limits:
  - Folders are listed with a few threads at once to hide network latency, but a first scan and the thumbnails still read every photo over the network.
  - Changes made from another computer may not be noticed while Picasso runs, because many network file systems don't report them. Press **F5** to rescan.
  - A share that drops out during a scan keeps its photos rather than removing them.
  - The library itself (`picasso.db`) must stay on a local disk. SQLite, which keeps it, can't lock files reliably over a network.
- **Cloud folders (iCloud Drive, OneDrive, Dropbox):** files kept only online are downloaded when Picasso reads them for the scan or a thumbnail. Keep the folder downloaded, or expect the first scan to download it.
