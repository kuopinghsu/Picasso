=== Picasso Places ===
Contributors: picasso
Tags: travel, map, geotag, itinerary, journey
Requires at least: 6.2
Tested up to: 7.1
Requires PHP: 7.4
Stable tag: 1.0.0
License: GPLv2 or later
License URI: https://www.gnu.org/licenses/gpl-2.0.html

Locations, route stops, opt-in maps and day-by-day Journeys for travel stories, with any theme.

== Description ==

Picasso Places brings the travel features of the VoyageLight theme to any theme, including maps, Journeys and the camera details Picasso keeps in a photo, and gives the Picasso photo app the fields it publishes travel stories to.

* **Location and route stops.** A post's editor has a Location field and a list of stops. A location can be typed as coordinates, in degrees and minutes, or pasted as a Google Maps, Apple Maps, OpenStreetMap or geo: link. The editor shows how it reads each location, and lists any line it can't read instead of dropping it. Places can be picked on a map or found by name.
* **Journeys.** A post type of its own, with day-by-day stops entered as rows or as `Day | Place | location | note | post ID` lines, and the stories that belong to it. A story chooses its Journey in the editor's Related journey box.
* **On the site.** After a post's content: a map of its location or stops, and a link to its Journey. On a Journey: its map, its stops day by day and its stories. Maps load only when the reader presses Load map. The styles take the theme's fonts and colours, in light and dark themes.
* **REST API.** `_vl_lat`, `_vl_lon`, `_vl_stops` and `_vl_trip_id` on posts and `_vl_trip_days` on Journeys (`/wp/v2/vl_trip`) are part of a published post, so anyone who can read the post can read them. Writing them needs a user who can edit the post. A missing coordinate reads as 0. Sending 0, 0 back with no stop line means no location. A stop line with those coordinates is a place at 0, 0.

The plugin is in English and Traditional Chinese, following Settings > General > Site Language.

== Installation ==

1. In Plugins > Add New > Upload Plugin, choose the plugin's ZIP file, then activate it.
2. With pretty permalinks, Journeys are at /trips/. If that address shows "not found", save Settings > Permalinks once.
3. Open a post: the Geotag & Travel Map box is under the content, and the Related journey box in the sidebar. Journeys have their own menu.

== Frequently Asked Questions ==

= Do I need this with the VoyageLight theme? =

No. VoyageLight has all of this built in. While VoyageLight, or a child theme of it, is active, the plugin does nothing at all, so it can stay installed, and active, for when you change theme.

= What happens to my locations and Journeys if I deactivate the plugin? =

They stay in the database. Under a theme without these features, Journeys and maps are no longer shown, and come back when the plugin is activated again.

= What do readers' browsers fetch from elsewhere? =

Nothing, until a reader presses Load map. Then the browser loads Leaflet from unpkg.com and map tiles from tile.openstreetmap.org, which see the reader's IP address and the page that asked. The plugin sets no cookies and keeps no data about readers.

= What does the editor fetch from elsewhere? =

Nothing, until an author asks. Pick on map loads Leaflet from unpkg.com and map tiles from tile.openstreetmap.org. Search sends the typed text, and the site's language, to OpenStreetMap Nominatim (nominatim.openstreetmap.org), one request per press. These services see the author's IP address.

= Are photo locations read from the pictures? =

No. A post's coordinates are the ones typed in its editor or sent by Picasso. Locations are shown to everyone who can read the post, so publish only places you are comfortable sharing.

= Why can't a short map link be read? =

Links such as maps.app.goo.gl hold no coordinates until they are opened. Open the link, then copy the full address from the browser's address bar.

== Privacy ==

The plugin stores the locations, stops and Journeys that authors enter, as post meta. It sends nothing anywhere by itself. A map (Load map, Pick on map) loads Leaflet 1.9.4 from unpkg.com, checked against a known hash, and OpenStreetMap tiles; Search in the editor queries OpenStreetMap Nominatim. Each happens only when someone presses the button, and those services receive that person's IP address. See the privacy policies of unpkg.com and the OpenStreetMap Foundation.

== Changelog ==

= 1.0.0 =
* First release: the location, stops and Journey fields and editor boxes of the VoyageLight theme 1.6.0, its REST API, and maps, itineraries and Journey links for any theme.

== Resources ==

* Leaflet 1.9.4, BSD 2-Clause License, loaded from unpkg.com only when a reader or author asks for a map; not included.
* Place search by OpenStreetMap Nominatim, used only when an author presses Search in the editor.
* Map data © OpenStreetMap contributors, Open Database License.
