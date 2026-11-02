# VoyageLight WordPress theme and Picasso Places plugin

English | [繁體中文](README.zh-TW.md)

An optional classic WordPress theme for a travel site, and an optional plugin that brings its locations, maps and Journeys to any other theme. Picasso doesn't need either: publishing works with any theme, and nothing in Picasso reads this folder.

| Path | What it is |
| --- | --- |
| `voyagelight/` | The theme's source, edited here |
| `voyagelight/languages/` | `voyagelight.pot`, the texts to translate, and `zh_TW.po`, their Traditional Chinese |
| `voyagelight/readme.txt` | The theme's own readme: description, FAQ, changelog, licences |
| `picasso-places/` | The Picasso Places plugin's source |
| `picasso-places/languages/` | `picasso-places.pot` and `picasso-places-zh_TW.po` |
| `picasso-places/readme.txt` | The plugin's own readme: description, FAQ, privacy, changelog |
| `setup-categories.sh` | Makes the theme's suggested categories, Destinations and Place types with WP-CLI |
| `tests/` | Tests for the location reader, in PHP and JavaScript, and for the plugin's guard |

## Build

```sh
make theme
```

This checks that every text in the PHP files has a translation in each `.po`, then writes `target/theme/voyagelight-<version>.zip` and `target/theme/picasso-places-<version>.zip`, with each `.po` compiled to a `.mo`. The theme's version comes from `voyagelight/style.css`, the plugin's from `picasso-places/picasso-places.php`, which must match its `const VERSION` and the Stable tag in its `readme.txt`. Building the same files gives the same ZIPs, byte for byte. `python3 scripts/build-theme.py --check` runs only the checks; CI runs it with `php -l` on every PHP file and the tests below.

The location reader has a PHP version (`voyagelight/inc/geo.php`) and a JavaScript one for the editor (`voyagelight/assets/js/geo.js`). Both are tested against the same cases in `tests/geo-cases.json`, in the theme and in the plugin:

```sh
php wordpress/tests/geo-test.php
node wordpress/tests/geo-test.js
php wordpress/tests/plugin-guard-test.php
```

The last one loads the plugin with stand-ins for WordPress and checks that it adds nothing while VoyageLight is the theme, that it adds its hooks under any other theme, and that none of its PHP names can clash with the theme's.

## Install

`make theme` writes `target/theme/voyagelight-<version>.zip` and `target/theme/picasso-places-<version>.zip`. The theme's version is the one in `voyagelight/style.css`, the plugin's the one in `picasso-places/picasso-places.php`. Both need WordPress 6.2 or later and PHP 7.4 or later, and were tried on WordPress 7.1 with PHP 8.5. Install the theme, or the plugin with any other theme. The plugin can stay active beside the theme: it does nothing while VoyageLight, or a child of it, is the theme.

### Theme

1. In WordPress, go to **Appearance > Themes > Add New > Upload Theme**, choose `voyagelight-<version>.zip` and activate it.
2. Save **Settings > Permalinks** once, so the `/trips/` and Destination addresses work.
3. Optionally, from the WordPress site's root, run `bash path/to/wordpress/setup-categories.sh`. It needs WP-CLI. It only adds the suggested categories, Destinations and Place types that are missing, so it can be run again.
4. Set the start page's headline and subtitle, and choose its photo from the media library, in **Appearance > Customize > VoyageLight homepage**. Left empty, the headline and subtitle follow the site's language and the theme's own photo is used. The same section turns camera details under photos off.

### Plugin

1. In **Plugins > Add Plugin > Upload Plugin**, choose `picasso-places-<version>.zip` and activate it.
2. Save **Settings > Permalinks** once if `/trips/` is not found.
3. A post then has **Geotag & Travel Map** under the content and **Related journey** in the sidebar. Journeys have their own menu.

## Languages

The theme's text is English, and Traditional Chinese when **Settings > General > Site Language** is 繁體中文. Posts are shown as written. The Destination tiles on the start page name each place in both languages whatever the site's language.

After adding or changing a text in the PHP, update the template and the translation:

```sh
cd wordpress/voyagelight
xgettext --language=PHP --from-code=UTF-8 --add-comments=translators: \
  --keyword=__ --keyword=_e --keyword=esc_html__ --keyword=esc_html_e \
  --keyword=esc_attr__ --keyword=esc_attr_e --keyword=_n:1,2 \
  --package-name=VoyageLight --package-version=1.6.0 -o languages/voyagelight.pot *.php inc/*.php
msgmerge --update --backup=none --no-fuzzy-matching languages/zh_TW.po languages/voyagelight.pot
```

Then translate the new entries in `zh_TW.po` and run `make theme`, which fails while one is missing.

The plugin works the same way with its own text domain, `picasso-places`. Its template also holds the plugin's name and description, so it is made with WP-CLI:

```sh
cd wordpress/picasso-places
wp i18n make-pot . languages/picasso-places.pot --skip-js --skip-block-json --skip-theme-json \
  --headers='{"Report-Msgid-Bugs-To":""}' \
  --file-comment='This file is distributed under the same license as the Picasso Places plugin.'
msgmerge --update --backup=none --no-fuzzy-matching \
  -C ../voyagelight/languages/zh_TW.po languages/picasso-places-zh_TW.po languages/picasso-places.pot
```

`-C` fills texts the plugin shares with the theme from the theme's translation, so the two use the same words: 旅程 for Journey, 遊記 for story, 停留點 for stop and 位置 for location.

## What it shows

- **Start page:** a large photo, six section tiles, the latest stories, Journeys, Destinations and an atlas of the posts that have a location.
- **Posts:** a location and a list of stops can be typed in a post's editor; the theme never reads them from photos. A location can be coordinates, degrees and minutes, or a Google Maps, Apple Maps, OpenStreetMap or geo: link; short links such as maps.app.goo.gl must be opened first. Lines the theme can't read are kept and listed in the editor. **Pick on map** loads Leaflet and OpenStreetMap tiles, and **Search** asks OpenStreetMap Nominatim, only when pressed. Wide and full-width images, galleries and columns are supported, and the block editor shows them in the site's fonts and column width. The line under the title is the post's excerpt, shown only when one is written.
- **Picasso's posts:** a photo sent with **Keep camera details** shows its camera, focal length, aperture, shutter speed and ISO under it, read by WordPress from the upload; photos in a gallery don't. Picasso's credits paragraph (`picasso-credits`) is set apart in smaller type.
- **Journeys:** a post type of its own with day-by-day stops and the stories that belong to them, entered in WordPress as rows or as `Day | Place | location | note | post ID` lines. **Add stops from linked stories** fills rows from the stories linked to the Journey. Picasso doesn't fill them in.
- **REST API:** `_vl_lat`, `_vl_lon`, `_vl_stops` and `_vl_trip_id` on posts and `_vl_trip_days` on Journeys are part of a published post, so anyone who can read the post can read them. Writing them needs a user who can edit the post. A missing coordinate reads as `0`. Sending `0, 0` back with no stop line means no location. A stop line with those coordinates is a place at 0, 0.
- **Maps:** nothing is fetched from elsewhere until a reader presses **Load map**; then Leaflet comes from unpkg.com and the tiles from OpenStreetMap. The fonts are part of the theme.

Journeys, Destinations, Place types and the map shortcodes (`[voyagelight_map]` and `[voyagelight_atlas]`) belong to the theme, so they are hidden while another theme is active and come back with it. The plugin brings Journeys and the post fields back under any theme, but not Destinations, Place types or the shortcodes.

## The plugin and the theme

Picasso Places registers the same post fields, the same Journey post type (`vl_trip`, at `/trips/`) and the same editor boxes as VoyageLight, with the same checks on what is saved, so posts and Journeys move between the two without changes. Under another theme it adds, after the post's text on a single post or Journey page:

- a **Places in this story** map and list of stops, on posts with a location or stops;
- **Part of the journey …**, on posts linked to a published Journey;
- a **Journey map**, the **Itinerary** day by day and the **Stories from this journey**, on Journeys;
- the camera, focal length, aperture, shutter speed and ISO under a photo Picasso sent with **Keep camera details**, including the featured image, and not under a photo in a gallery.

It leaves feeds, excerpts, archives and the REST API alone, and its styles take their colours and fonts from the theme.

Picasso reads which fields a site's posts take before it publishes. With the theme or the plugin, it sends a Story's Places as these fields and can make a Journey. With neither, it puts the Places at the end of the post instead, as a map picture with numbered pins and a list linked to OpenStreetMap, and saves the posts without a Journey.

While VoyageLight is the active theme the plugin does nothing at all, so the two can be active together and switching themes keeps everything. Plugins load before themes, so the plugin can't define any PHP name the theme does: its PHP is in the `PicassoPlaces` namespace, and its script handles, form fields and CSS classes start with `picasso-places` or `picasso_places`. The ids, classes and `vlGeo`, `vlGeoAdmin` and `vlText` objects that the shared scripts look for keep their `vl` names. Picasso's own posts use `picasso-places` and `picasso-places-map` for the static map Picasso draws when a site has no location fields; with the theme or the plugin active, Picasso sends the fields instead and draws no static map.

The location reader and the editor and map scripts are shared rather than rewritten, so the two can't read a location differently. The theme's files are the ones to edit; the plugin keeps copies so that its folder works as it is:

| Theme file | Plugin copy |
| --- | --- |
| `voyagelight/inc/geo.php` | `picasso-places/inc/geo.php`, with `namespace PicassoPlaces;` added |
| `voyagelight/assets/js/geo.js` | `picasso-places/assets/js/geo.js` |
| `voyagelight/assets/js/geo-admin.js` | `picasso-places/assets/js/geo-admin.js` |
| `voyagelight/assets/js/map.js` | `picasso-places/assets/js/map.js` |
| `voyagelight/assets/css/geo-admin.css` | `picasso-places/assets/css/geo-admin.css` |

After changing one of the theme's files, run `python3 scripts/build-theme.py --sync` to update the copies. `--check`, and so `make theme` and CI, fails while a copy differs.

## Licences

The theme is GPLv2 or later, and so are its illustrations and demo photos; the fonts are under the SIL Open Font License. `voyagelight/readme.txt` lists each one. The plugin is GPLv2 or later; the map library it loads, Leaflet, is BSD-2-Clause.
