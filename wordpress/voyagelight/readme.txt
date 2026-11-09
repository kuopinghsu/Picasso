=== VoyageLight ===
Contributors: voyagelight
Requires at least: 6.2
Tested up to: 7.1
Requires PHP: 7.4
Stable tag: 1.7.0
License: GPLv2 or later
License URI: https://www.gnu.org/licenses/gpl-2.0.html

A photo-led travel journal with journeys, day-by-day itineraries, destinations, geotags and opt-in maps.

== Description ==

VoyageLight is a classic theme for travel stories. The homepage leads with a large photo, six photo tiles, the latest stories, journeys, destinations and a travel atlas. Posts can carry a location and a list of stops, shown on a map only after the reader chooses to load it. Journeys gather day-by-day stops and the stories that belong to them.

A location can be typed as coordinates, in degrees and minutes, or pasted as a Google Maps, Apple Maps, OpenStreetMap or geo: link. The editor shows how it reads each location, and lists any line it can't read instead of dropping it. Places can also be picked on a map or found by name, and a Journey's itinerary can be edited as rows.

The theme comes in two builds, English (voyagelight-<version>-en.zip) and Traditional Chinese (voyagelight-<version>-zh-TW.zip), each in its language whatever the site's; set Settings > General > Site Language to match. Posts are shown as written; nothing is translated automatically.

Gutenberg's wide and full-width images, galleries and columns are supported, and the block editor shows posts in the site's fonts and column width. A photo uploaded with its camera details (camera, focal length, aperture, shutter speed and ISO) shows them under it, unless turned off in Appearance > Customize > VoyageLight homepage; photos in a gallery don't.

== Installation ==

1. In Appearance > Themes > Add New > Upload Theme, choose the theme's ZIP file and activate it.
2. Save Settings > Permalinks once, so the /trips/ and taxonomy addresses work.
3. Set the homepage headline, subtitle and photo in Appearance > Customize > VoyageLight homepage; the photo is chosen from the media library. Left empty, the headline and subtitle are the theme's own. Change any of the theme's words in Appearance > Customize > VoyageLight text, one per line as `shown text = new text`; categories, Destinations and Place types are made and renamed under Posts.

== Frequently Asked Questions ==

= What do readers' browsers fetch from elsewhere? =

Nothing, until a reader presses Load map. Then the browser loads map tiles from tile.openstreetmap.org, which see the reader's IP address. The fonts and the map library, Leaflet, are part of the theme.

= What does the editor fetch from elsewhere? =

Nothing, until an author asks. Pick on map loads map tiles from tile.openstreetmap.org. Search sends the typed text to OpenStreetMap Nominatim, one request per press. These services see the author's IP address.

= Why can't a short map link be read? =

Links such as maps.app.goo.gl hold no coordinates until they are opened. Open the link, then copy the full address from the browser's address bar.

= Are photo locations read from the pictures? =

No. A post's coordinates are the ones typed in its editor. Publish only places you are comfortable sharing.

= What happens to Journeys if I change theme? =

Journeys, Destinations, Place types and the map shortcodes belong to this theme. Under another theme they stay in the database but are no longer shown, and the shortcodes appear as plain text. Switching back brings them back.

== Changelog ==

= 1.7.0 =
* A modern redesign: a sticky, translucent header with pill navigation; a rounded photo hero; cards that lift on hover; portrait destination tiles; a cleaner sidebar; and refreshed articles, maps and Journey pages, in light and dark.
* Articles read in DM Sans, with Playfair Display headings and Chinese fallback fonts.
* The sidebar lists categories as a nested tree with post counts, and the header menu lists them with their sub-categories. On the start page the latest stories sit in two columns beside the categories, which become a row of chips above the stories on a phone, where stories after the newest are compact rows.
* Once a site has three top-level categories, the photo tiles under the hero are those categories, each with its latest story's featured image.
* The footer lists the top categories and the latest Journeys.
* Maps stay short until they are loaded, and the Travel atlas no longer repeats its section's heading. An article keeps to two widths: 720px for reading and 1100px for the Journey explorer and story grids.
* Tables in light and dark: a fjord-teal header row with a brass rule, a sand totals row, sea-glass stripes for the Stripes style, column alignment from the Table block, and sideways scrolling on a phone with a soft shadow on the edge that has more; the block editor shows them the same way. A table pasted outside a Table block gets the same frame, a first row of header cells is marked as column headers, and a table that scrolls can be reached and scrolled with the keyboard. Tables published from Picasso keep the column widths chosen there.
* An embedded YouTube video starts where its address says, such as t=90s or t=1m30s.
* On a phone the start page's tiles are one row to swipe and its buttons full width, sub-categories are lighter chips, and a story ends with a brass rule, its details in one card and Previous and Next side by side with the stories' titles; featured images keep a 4:3 minimum.
* Every word the theme shows can be changed in Appearance > Customize > VoyageLight text, without a plugin or editing files. The Featured destinations tiles follow Posts > Destinations once three have stories, the logo follows Site Identity, and the photo beside "Every place has a story" can be chosen.
* Widget areas for the sidebar and the footer, with a VoyageLight panel widget for the theme's own panels; each shows the theme's content until a widget is added.
* The start page's sections can be turned off one by one, with how many stories and Journeys it lists; an accent colour of any shade sets links, buttons and tables, made readable in light and dark; and a text changed to nothing is hidden.
* Link previews for social apps (Open Graph) and structured data for search engines (JSON-LD), with a Journey's itinerary and a story's places; a password-protected post shares only its title, and an active SEO plugin takes over both.
* Breadcrumbs above titles, related stories after a story, a Start page sidebar widget area, and address bases for Journeys, Destinations and Place types in Settings > Permalinks.
* The start page's sections can be put in any order below the large photo; how many related stories and footer Journeys are listed, and how long card summaries are, can be set in Appearance > Customize > VoyageLight stories. Journey cards now use the same summary length as story cards.
* On a new site, the Search, Recent Posts and other widgets WordPress adds on install move to Inactive widgets when the theme is turned on, so the category tree shows; an area with any other widget is left as it is. Photo tiles side by side no longer show the same story's photo twice, password-protected stories' photos are never used for them, and an empty Place types or Destinations panel is left out.
* A search with no results shows a note, the search box and the travel categories; the footer sits at the bottom of short pages; Featured destinations take as many columns as there are tiles, and an odd last tile on a phone takes the row; the newest story is the large card only when it has a photo; links in a Journey's stop list are in the accent colour; and breadcrumbs stay on one line on a phone, with larger tap targets.
* Light or dark in Appearance > Customize > Colours: as the visitor's device is set, light or dark, with or without the header's button to switch; a fixed choice without the button is written into the page, so it holds without the script. The sidebar of story lists and the start page's latest stories goes on the right, on the left or nowhere, in Appearance > Customize > VoyageLight stories; on a phone it stays after the stories.
* English and Traditional Chinese builds: make theme writes voyagelight-<version>-en.zip and voyagelight-<version>-zh-TW.zip to wordpress/, each in its language whatever the site's, without the translators' .po and .pot files.
* setup-categories.sh is gone: categories, Destinations and Place types are made in the Posts menu. Until the site has its own categories, the Food & Culture and Travel Guides tiles lead to all stories.
* Easier to use: the light and dark button is named by its label and shows whether dark is on; a header menu's sub-menus open with a button, for touch screens and keyboards, and the header turns to the phone menu whenever its links don't fit; a loaded map takes the keyboard focus; and the current category is marked for screen readers. Section labels are written in normal case and capitalised by the stylesheet, and a change in VoyageLight text matches a text whatever its case. Chinese, Japanese and Korean headings keep their own letter spacing; the start page's photo is an image that a phone fetches at its own size; cards without a picture take one of six illustrations; photo tiles that would only repeat the stories link are left out; and the icons are drawn, not characters.
* Leaflet comes with the theme, so a map fetches only OpenStreetMap's tiles. functions.php is split into files in inc/ by subject.
* Form fields have borders with 3:1 contrast, small text is at least 12px, and sidebar rows and day tabs are about 44px tall on a phone.
* A password-protected story or Journey keeps its places hidden, on its page, the Travel atlas and the REST API, until the password is entered.
* Map pins show titles as written, with quotes and ampersands.

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
* Leaflet 1.9.4, Copyright 2010-2023 Volodymyr Agafonkin, BSD 2-Clause License, assets/vendor/leaflet/LICENSE; loaded only when a reader or author asks for a map.
* Place search by OpenStreetMap Nominatim, used only when an author presses Search in the editor.
* Map data © OpenStreetMap contributors, Open Database License.
