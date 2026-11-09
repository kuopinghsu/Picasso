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
| `tests/` | Tests for the location reader, in PHP and JavaScript, and for the plugin's guard |

## Build

```sh
make theme
```

This checks that every text in the PHP files has a translation in each `.po` and that the `.pot` and `.po` files are up to date with the PHP (see Languages), then writes an English and a Traditional Chinese build of each to `wordpress/`: `voyagelight-<version>-en.zip`, `voyagelight-<version>-zh-TW.zip`, `picasso-places-<version>-en.zip` and `picasso-places-<version>-zh-TW.zip`. A build shows its language whatever the site's; the Chinese one carries its `.po` compiled to a `.mo`, and neither carries the `.po` or `.pot` files. The theme's version comes from `voyagelight/style.css`, the plugin's from `picasso-places/picasso-places.php`, which must match its `const VERSION` and the Stable tag in its `readme.txt`. Building the same files gives the same ZIPs, byte for byte. `python3 scripts/build-theme.py --check` runs only the checks; CI runs it with `php -l` on every PHP file and the tests below.

The location reader has a PHP version (`voyagelight/inc/geo.php`) and a JavaScript one for the editor (`voyagelight/assets/js/geo.js`). Both are tested against the same cases in `tests/geo-cases.json`, in the theme and in the plugin:

```sh
php wordpress/tests/geo-test.php
node wordpress/tests/geo-test.js
php wordpress/tests/plugin-guard-test.php
```

The last one loads the plugin with stand-ins for WordPress and checks that it adds nothing while VoyageLight is the theme, that it adds its hooks under any other theme, and that none of its PHP names can clash with the theme's.

## Install

Choose the language when choosing the ZIP in `wordpress/`: `-en` for English, `-zh-TW` for Traditional Chinese. The theme's words, its Customizer settings and the plugin's are then in that language whatever **Settings > General > Site Language** says; set the site's language to match, so WordPress's own words, such as dates and the comment form, are in it too. To change language later, upload the other ZIP: the settings, menus and widgets stay. The theme's version is the one in `voyagelight/style.css`, the plugin's the one in `picasso-places/picasso-places.php`. Both need PHP 7.4 or later, and WordPress 6.2 or later for the theme and 6.4 or later for the plugin, and were tried on WordPress 7.1 with PHP 8.5. Install the theme, or the plugin with any other theme. The plugin can stay active beside the theme: it does nothing while VoyageLight, or a child of it, is the theme.

### Theme

1. In WordPress, go to **Appearance > Themes > Add New > Upload Theme**, choose `voyagelight-<version>-en.zip` or `voyagelight-<version>-zh-TW.zip` and activate it.
2. Save **Settings > Permalinks** once, so the `/trips/` and Destination addresses work.
3. Make categories, Destinations and Place types in **Posts > Categories**, **Posts > Destinations** and **Posts > Place types**, where they can be renamed, moved under another and deleted at any time. The sidebar, the header menu, the footer and the start page's tiles follow them.
4. Set the start page's headline and subtitle, and choose its photo from the media library, in **Appearance > Customize > VoyageLight homepage**. Left empty, the headline and subtitle are the theme's own, in its language, and so is the photo. The same section turns camera details under photos off. **Collections photo** there sets the photo beside "Every place has a story", and **Site Identity** sets the logo, the site's name and its tagline.
5. Change any of the theme's own words in **Appearance > Customize > VoyageLight text**, with no plugin and no files: one change per line, the text as the site shows it, `=`, then the new text, such as `Latest travel stories = Recent posts`, or `最新旅行故事 = 最新文章` on a Chinese site. A text with `%s` or `%d` keeps them; a change that drops or adds one is ignored. The changes are kept in the site's settings, so updating the theme doesn't undo them. Nothing after the `=` hides the text, such as `Powered by WordPress =`.
6. In **Appearance > Customize > VoyageLight homepage**, turn each section of the start page on or off, move them up or down under **Order of sections** (the large photo stays first), and set how many stories and Journeys it lists. In **Appearance > Customize > Colours**, choose the accent colour: the theme makes the shades for links, buttons, tables and dark mode from it, all readable. **Light or dark** there sets the colours a visitor first sees: as their device is set, light or dark; **Let visitors switch between light and dark** keeps the header's button, whose choice is kept in the visitor's browser.
7. In **Appearance > Widgets**, add WordPress's own widgets (About, Latest Posts, Search…) to **Sidebar** and **Footer**; the **VoyageLight panel** widget brings back any of the theme's panels. While an area has no widgets, it shows the theme's own content. The widgets WordPress puts in the first area of a new site (Search, Recent Posts, Recent Comments, Archives, Categories) move to **Inactive widgets** when the theme is turned on, unless the area holds other widgets too; drag them back to use them. **Start page sidebar** holds what sits beside the latest stories on the start page.
8. In **Settings > Permalinks**, under Optional, change the address bases of Journeys (`trips`), Destinations (`destination`) and Place types (`place-type`), such as `journeys`. In **Appearance > Customize > VoyageLight stories**, put the sidebar of story lists and the start page's latest stories on the right, on the left or nowhere; turn the breadcrumbs above titles and the related stories after a story on or off, and set how many related stories and footer Journeys are listed and how long card summaries are (in words; WordPress counts characters in Chinese and Japanese).

Each page also carries a link preview for social apps (Open Graph: title, summary and photo) and structured data for search engines (JSON-LD: the story with its places, the Journey with its itinerary, and the breadcrumb trail). A password-protected post shares only its title. With an SEO plugin active, the theme leaves both to it.

### Plugin

1. In **Plugins > Add Plugin > Upload Plugin**, choose `picasso-places-<version>-en.zip` or `picasso-places-<version>-zh-TW.zip` and activate it.
2. Save **Settings > Permalinks** once if `/trips/` is not found.
3. A post then has **Geotag & Travel Map** under the content and **Related journey** in the sidebar. Journeys have their own menu.

## Languages

The theme and the plugin come in English and in Traditional Chinese, as separate ZIPs (see Install); a site uses one or the other. Posts are shown as written. The Destination tiles on the start page name each place in both languages whatever the site's language.

Running a site needs none of what follows, only the ZIP in its language: any text the theme shows can be changed, or hidden, in **Appearance > Customize > VoyageLight text** (step 5 above), and categories, menus, widgets and posts are made in WordPress itself.

Only someone changing the theme's or plugin's PHP needs this. After adding or changing a text there, run

```sh
make translations
```

The source in `wordpress/voyagelight` and `wordpress/picasso-places`, which has no `inc/language.php`, follows the site's language instead, for trying translations. `make translations` rewrites `languages/voyagelight.pot` and `languages/picasso-places.pot` from the PHP, with each text's plural and `translators:` comment, and brings each `.po` in line: a new text is added with an empty translation, and a text the PHP no longer uses is removed. A new plugin text the theme also has starts with the theme's translation, so the two use the same words: 旅程 for Journey, 遊記 for story, 停留點 for stop and 位置 for location. It lists the texts left to translate; fill them in in the `.po`, then run `make theme`, which fails while a translation is missing or a `.pot` or `.po` is out of date with the PHP. No gettext tools or WP-CLI are needed.

## What it shows

- **Start page:** a large photo, photo tiles for its sections or the site's top categories, the latest stories, Journeys, Destinations and an atlas of the posts that have a location.
- **Posts:** a location and a list of stops can be typed in a post's editor; the theme never reads them from photos. A location can be coordinates, degrees and minutes, or a Google Maps, Apple Maps, OpenStreetMap or geo: link; short links such as maps.app.goo.gl must be opened first. Lines the theme can't read are kept and listed in the editor. **Pick on map** loads Leaflet and OpenStreetMap tiles, and **Search** asks OpenStreetMap Nominatim, only when pressed. Wide and full-width images, galleries and columns are supported, and the block editor shows them in the site's fonts and column width. The line under the title is the post's excerpt, shown only when one is written.
- **Picasso's posts:** a photo sent with **Keep camera details** shows its camera, focal length, aperture, shutter speed and ISO under it, read by WordPress from the upload; photos in a gallery don't. Picasso's credits paragraph (`picasso-credits`) is set apart in smaller type.
- **Journeys:** a post type of its own with day-by-day stops and the stories that belong to them, entered in WordPress as rows or as `Day | Place | location | note | post ID` lines. **Add stops from linked stories** fills rows from the stories linked to the Journey. Picasso doesn't fill them in.
- **REST API:** `_vl_lat`, `_vl_lon`, `_vl_stops` and `_vl_trip_id` on posts and `_vl_trip_days` on Journeys are part of a published post, so anyone who can read the post can read them. Writing them needs a user who can edit the post. A missing coordinate reads as `0`. Sending `0, 0` back with no stop line means no location. A stop line with those coordinates is a place at 0, 0.
- **Maps:** nothing is fetched from elsewhere until a reader presses **Load map**; then the tiles come from OpenStreetMap. The fonts and Leaflet, the map library, are part of the theme and the plugin.

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
| `voyagelight/assets/vendor/leaflet/` | `picasso-places/assets/vendor/leaflet/` |

After changing one of the theme's files, run `python3 scripts/build-theme.py --sync` to update the copies. `--check`, and so `make theme` and CI, fails while a copy differs.

## Licences

The theme is GPLv2 or later, and so are its illustrations and demo photos; the fonts are under the SIL Open Font License. `voyagelight/readme.txt` lists each one. The plugin is GPLv2 or later; the map library it loads, Leaflet, is BSD-2-Clause.
