=== VoyageLight ===
Contributors: voyagelight
Requires at least: 6.2
Tested up to: 7.1
Requires PHP: 7.4
Stable tag: 1.6.0
License: GPLv2 or later
License URI: https://www.gnu.org/licenses/gpl-2.0.html

A photo-led travel journal with journeys, day-by-day itineraries, destinations, geotags and opt-in maps.

== Description ==

VoyageLight is a classic theme for travel stories. The homepage leads with a large photo, six photo tiles, the latest stories, journeys, destinations and a travel atlas. Posts can carry a location and a list of stops, shown on a map only after the reader chooses to load it. Journeys gather day-by-day stops and the stories that belong to them.

A location can be typed as coordinates, in degrees and minutes, or pasted as a Google Maps, Apple Maps, OpenStreetMap or geo: link. The editor shows how it reads each location, and lists any line it can't read instead of dropping it. Places can also be picked on a map or found by name, and a Journey's itinerary can be edited as rows.

The theme follows the site's language: English, and Traditional Chinese when Settings > General > Site Language is 繁體中文. Posts are shown as written; nothing is translated automatically.

Gutenberg's wide and full-width images, galleries and columns are supported, and the block editor shows posts in the site's fonts and column width. A photo uploaded with its camera details (camera, focal length, aperture, shutter speed and ISO) shows them under it, unless turned off in Appearance > Customize > VoyageLight homepage; photos in a gallery don't.

== Installation ==

1. In Appearance > Themes > Add New > Upload Theme, choose the theme's ZIP file and activate it.
2. Save Settings > Permalinks once, so the /trips/ and taxonomy addresses work.
3. Set the homepage headline, subtitle and photo in Appearance > Customize > VoyageLight homepage; the photo is chosen from the media library. Left empty, the headline and subtitle follow the site's language.

== Frequently Asked Questions ==

= What do readers' browsers fetch from elsewhere? =

Nothing, until a reader presses Load map. Then the browser loads Leaflet from unpkg.com and map tiles from tile.openstreetmap.org, which see the reader's IP address. The fonts are part of the theme.

= What does the editor fetch from elsewhere? =

Nothing, until an author asks. Pick on map loads Leaflet from unpkg.com and map tiles from tile.openstreetmap.org. Search sends the typed text to OpenStreetMap Nominatim, one request per press. These services see the author's IP address.

= Why can't a short map link be read? =

Links such as maps.app.goo.gl hold no coordinates until they are opened. Open the link, then copy the full address from the browser's address bar.

= Are photo locations read from the pictures? =

No. A post's coordinates are the ones typed in its editor. Publish only places you are comfortable sharing.

= What happens to Journeys if I change theme? =

Journeys, Destinations, Place types and the map shortcodes belong to this theme. Under another theme they stay in the database but are no longer shown, and the shortcodes appear as plain text. Switching back brings them back.

== Changelog ==

= 1.6.0 =
* One theme for English and Traditional Chinese, following the site's language.
* Fonts served from the theme instead of Google Fonts.
* Styles for wide, full-width and floated blocks.
* Destination tiles, sample stories and posts without a featured image use the theme's illustrations instead of demo photos with text in them; the text is cropped off the homepage photo.
* Camera details under photos that carry them, and a style for Picasso's photo credits.
* The line under a post's title shows only a written excerpt, and Destination, Place type and Tags only when set.
* Dark mode is set before the page is drawn, so it no longer flashes light.
* Each story card is one link; the travel atlas reads its posts in one query; the homepage photo is fetched first and chosen from the media library.
* The block editor uses the site's fonts and column width.
* Removed the "Trips / Journeys" tags, which duplicated a story's Related journey; the sidebar lists Journeys instead.
* Fixed the Dark/Light button's label and reading time for Chinese posts.
* One Location field replaces Latitude and Longitude. It reads decimal coordinates, degrees and minutes, and Google Maps, Apple Maps, OpenStreetMap and geo: links, and shows the reading as you type.
* Route stops are written Place | location | note, and Journey lines Day | Place | location | note | post ID. The old formats still work.
* Lines that can't be read are kept as typed and listed in the editor, with the reason.
* Pick on map and Find a place in the post and Journey editors. Pins can be dragged.
* A Journey's itinerary can be edited as rows, and filled from the stories linked to it.
* Locations, stops and itineraries can be read and written through the REST API.
* Fixed the map stylesheet's integrity hash, which stopped Load map from working.
* Fixed a Journey's list of stops numbering each stop twice.

== Copyright ==

VoyageLight WordPress Theme, Copyright 2026 VoyageLight Studio.
VoyageLight is distributed under the terms of the GNU GPL, version 2 or later.

This program is free software: you can redistribute it and/or modify it under the terms of the GNU General Public License as published by the Free Software Foundation, either version 2 of the License, or (at your option) any later version.

This program is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the GNU General Public License for more details.

== Resources ==

* DM Sans, Copyright 2014 The DM Sans Project Authors. SIL Open Font License 1.1, assets/fonts/OFL-DMSans.txt.
* Playfair Display, Copyright 2017 The Playfair Display Project Authors. SIL Open Font License 1.1, assets/fonts/OFL-PlayfairDisplay.txt.
* Logo and destination illustrations in assets/images/*.svg: original artwork of the theme, GPLv2 or later.
* Demo photographs in assets/images/photos: AI-generated for this theme, not photographs of real places, GPLv2 or later.
* Leaflet 1.9.4, BSD 2-Clause License, loaded from unpkg.com only when a reader or author asks for a map; not included.
* Place search by OpenStreetMap Nominatim, used only when an author presses Search in the editor.
* Map data © OpenStreetMap contributors, Open Database License.
