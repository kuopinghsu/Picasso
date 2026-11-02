#!/usr/bin/env bash
# Run from WordPress root with WP-CLI after theme activation. Creates missing terms, never deletes content.
set -euo pipefail
ensure(){ local tax="$1" name="$2" slug="$3" parent="${4:-}"; local args=(--slug="$slug"); if [[ -n "$parent" ]];then args+=(--parent="$(wp term get "$tax" "$parent" --field=term_id --by=slug)");fi; wp term get "$tax" "$slug" --by=slug --field=term_id >/dev/null 2>&1 || wp term create "$tax" "$name" "${args[@]}" --porcelain >/dev/null; }
# Editorial content type: keep broad, avoid creating competing parallel hierarchies.
for item in 'Travel Stories|travel-stories' 'Itineraries|itineraries' 'Travel Guides|travel-guides' 'Food & Culture|food-culture' 'Places to Stay|places-to-stay' 'Travel Notes|travel-notes';do IFS='|' read -r name slug <<< "$item";ensure category "$name" "$slug";done
# Destinations by continent > region > country > city. The start page's tiles link to nordic, greenland, baltic, spain and japan.
for item in 'Europe|europe|' 'Asia|asia|' 'Arctic|arctic|' 'Nordic|nordic|europe' 'Baltic|baltic|europe' 'Spain|spain|europe' 'Norway|norway|nordic' 'Denmark|denmark|nordic' 'Latvia|latvia|baltic' 'Estonia|estonia|baltic' 'Lithuania|lithuania|baltic' 'Japan|japan|asia' 'Taiwan|taiwan|asia' 'Greenland|greenland|arctic' 'Svalbard|svalbard|arctic' 'Tokyo|tokyo|japan' 'Toledo|toledo|spain' 'Riga|riga|latvia' 'Tallinn|tallinn|estonia' 'Vilnius|vilnius|lithuania' 'Ilulissat|ilulissat|greenland' 'Longyearbyen|longyearbyen|svalbard';do IFS='|' read -r name slug parent <<< "$item";ensure destination "$name" "$slug" "$parent";done
# Place characteristics help readers filter types of stop; do not duplicate content categories.
for item in 'Natural Wonders|natural-wonders' 'Landmarks & Heritage|landmarks-heritage' 'Museums & Galleries|museums-galleries' 'Neighborhoods & Walking|neighborhoods-walking' 'Food & Markets|food-markets' 'Hotels & Lodging|hotels-lodging' 'Viewpoints|viewpoints' 'Outdoor Activities|outdoor-activities' 'Transportation Hubs|transportation-hubs';do IFS='|' read -r name slug <<< "$item";ensure travel_type "$name" "$slug";done
wp rewrite flush
printf 'Travel categories created. Existing categories and posts were not removed. Review any legacy categories manually.\n'
