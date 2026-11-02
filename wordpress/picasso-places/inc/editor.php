<?php
/**
 * The geotag boxes in the story and Journey editors. The ids and vl- classes
 * are the ones assets/js/geo-admin.js, shared with the VoyageLight theme, looks for.
 *
 * @package PicassoPlaces
 */

namespace PicassoPlaces;

if ( ! defined( 'ABSPATH' ) ) {
	exit;
}

/** Why a location or a line can't be read, keyed by the reasons vl_geo_read() and the line readers give. */
function geo_reasons() {
	return array(
		'empty'      => __( 'There is no location.', 'picasso-places' ),
		'short-link' => __( 'Short links can’t be read without opening them. Open the link, then copy the full address from the browser’s address bar.', 'picasso-places' ),
		'link'       => __( 'This link has no coordinates in it. Open the place on the map, then copy the address again.', 'picasso-places' ),
		'range'      => __( 'Out of range: latitude goes from -90 to 90 and longitude from -180 to 180.', 'picasso-places' ),
		'unreadable' => __( 'This is not a location that can be read.', 'picasso-places' ),
		'name'       => __( 'The place name is missing.', 'picasso-places' ),
		/* translators: %d: highest day number. */
		'day'        => sprintf( __( 'The day must be a number from 1 to %d.', 'picasso-places' ), VL_GEO_DAY_MAX ),
	);
}

/** A notice naming each line of `$text` that can't be read, or '' when all can. */
function geo_problem_list( $text, $kind, $max_lines ) {
	$problems = vl_geo_problems( $text, $kind, $max_lines );
	if ( ! $problems ) {
		return '';
	}
	$reasons = geo_reasons();
	$out     = '<div class="notice notice-warning inline"><p>' . esc_html__( 'These lines couldn’t be read and are not on the map:', 'picasso-places' ) . '</p><ul>';
	foreach ( $problems as $problem ) {
		/* translators: 1: line number, 2: the text of the line, 3: why it couldn't be read. */
		$out .= '<li>' . esc_html( sprintf( __( 'Line %1$d: “%2$s” — %3$s', 'picasso-places' ), $problem[0], $problem[1], $reasons[ $problem[2] ] ) ) . '</li>';
	}
	return $out . '</ul></div>';
}

/** The Pick on map panel; assets/js/geo-admin.js makes it work. */
function geo_picker( $kind ) {
	$panel = 'vl-geo-panel-' . $kind;
	echo '<div class="vl-geo-picker hide-if-no-js" data-kind="' . esc_attr( $kind ) . '">';
	echo '<p><button type="button" class="button vl-geo-open" aria-expanded="false" aria-controls="' . esc_attr( $panel ) . '">' . esc_html__( 'Pick on map', 'picasso-places' ) . '</button> ';
	echo '<span class="description">' . esc_html__( 'The map loads Leaflet from unpkg.com and map tiles from OpenStreetMap, which see your IP address.', 'picasso-places' ) . '</span></p>';
	echo '<div class="vl-geo-panel" id="' . esc_attr( $panel ) . '" hidden><div class="vl-geo-search">';
	echo '<label for="vl-geo-query-' . esc_attr( $kind ) . '">' . esc_html__( 'Find a place', 'picasso-places' ) . '</label> ';
	echo '<input type="search" id="vl-geo-query-' . esc_attr( $kind ) . '" class="vl-geo-query regular-text" autocomplete="off"> ';
	echo '<button type="button" class="button vl-geo-find">' . esc_html__( 'Search', 'picasso-places' ) . '</button>';
	echo '<p class="description">' . esc_html__( 'Search by OpenStreetMap Nominatim', 'picasso-places' ) . ' · ' . esc_html__( 'Pressing Search sends what you typed to it.', 'picasso-places' ) . '</p>';
	echo '<p class="vl-geo-search-status" aria-live="polite"></p><ul class="vl-geo-results"></ul></div>';
	if ( 'post' === $kind ) {
		echo '<p><label>' . esc_html__( 'A click on the map', 'picasso-places' ) . ' <select class="vl-geo-click">';
		echo '<option value="location">' . esc_html__( 'sets the story’s location', 'picasso-places' ) . '</option>';
		echo '<option value="stop">' . esc_html__( 'adds a route stop', 'picasso-places' ) . '</option></select></label></p>';
	}
	echo '<p class="vl-geo-map-status" aria-live="polite"></p><div class="vl-geo-map"></div>';
	echo '<div class="vl-geo-add" hidden><label>' . esc_html__( 'Place name', 'picasso-places' ) . ' <input type="text" class="vl-geo-add-name"></label>';
	if ( 'trip' === $kind ) {
		echo ' <label>' . esc_html__( 'Day', 'picasso-places' ) . ' <input type="number" class="vl-geo-add-day small-text" min="1" max="' . esc_attr( VL_GEO_DAY_MAX ) . '"></label>';
	}
	echo ' <button type="button" class="button button-primary vl-geo-add-ok">' . esc_html__( 'Add stop', 'picasso-places' ) . '</button>';
	echo ' <button type="button" class="button vl-geo-add-cancel">' . esc_html__( 'Cancel', 'picasso-places' ) . '</button></div>';
	echo '<p class="description">' . ( 'post' === $kind ? esc_html__( 'Click the map to set the location or add a stop. Drag a pin to move it.', 'picasso-places' ) : esc_html__( 'Click the map to add a stop. Drag a pin to move it.', 'picasso-places' ) ) . '</p>';
	echo '</div></div>';
}

function editor_boxes() {
	add_meta_box( 'picasso-places-geo', __( 'Geotag & Travel Map', 'picasso-places' ), __NAMESPACE__ . '\\geo_fields', 'post', 'normal', 'high' );
	add_meta_box( 'picasso-places-trip-days', __( 'Journey itinerary & GeoTags', 'picasso-places' ), __NAMESPACE__ . '\\trip_days_box', 'vl_trip', 'normal', 'high' );
	add_meta_box( 'picasso-places-trip-link', __( 'Related journey', 'picasso-places' ), __NAMESPACE__ . '\\trip_link_box', 'post', 'side', 'high' );
}
add_action( 'add_meta_boxes', __NAMESPACE__ . '\\editor_boxes' );

function geo_fields( $post ) {
	wp_nonce_field( 'picasso_places_geo_save', 'picasso_places_geo_nonce' );
	$lat     = vl_valid_coord( get_post_meta( $post->ID, '_vl_lat', true ), 'lat' );
	$lon     = vl_valid_coord( get_post_meta( $post->ID, '_vl_lon', true ), 'lon' );
	$stops   = get_post_meta( $post->ID, '_vl_stops', true );
	$unread  = get_transient( 'picasso_places_location_' . $post->ID . '_' . get_current_user_id() );
	$reasons = geo_reasons();
	$value   = null !== $lat && null !== $lon ? $lat . ', ' . $lon : '';
	echo '<p><label for="vl_location"><strong>' . esc_html__( 'Location', 'picasso-places' ) . '</strong></label> · ' . esc_html__( 'Paste coordinates or a map link, such as 39.8628, -4.0273 or 39°51′46″N 4°01′38″W. Links from Google Maps, Apple Maps and OpenStreetMap and geo: addresses work too. Leave it empty for no location.', 'picasso-places' ) . '</p>';
	if ( is_array( $unread ) && isset( $unread['text'], $reasons[ $unread['why'] ] ) ) {
		$value = $unread['text'];
		/* translators: 1: the location as typed, 2: why it couldn't be read. */
		echo '<div class="notice notice-warning inline"><p>' . esc_html( sprintf( __( 'The location “%1$s” couldn’t be read: %2$s The saved location was kept.', 'picasso-places' ), $unread['text'], $reasons[ $unread['why'] ] ) ) . '</p></div>';
	}
	$point = vl_geo_read( $value );
	if ( is_array( $point ) ) {
		/* translators: %s: latitude and longitude, such as 39.86280, -4.02730. */
		$reading = sprintf( __( 'Reads as %s', 'picasso-places' ), sprintf( '%.5F, %.5F', $point[0], $point[1] ) );
	} else {
		$reading = 'empty' === $point ? __( 'No location set.', 'picasso-places' ) : $reasons[ $point ];
	}
	echo '<input type="text" id="vl_location" name="picasso_places_location" class="widefat" value="' . esc_attr( $value ) . '" autocomplete="off" spellcheck="false" aria-describedby="vl-location-reading vl-location-help">';
	echo '<p id="vl-location-reading" class="vl-geo-reading' . ( is_string( $point ) && 'empty' !== $point ? ' is-error' : '' ) . '" aria-live="polite">' . esc_html( $reading ) . '</p>';
	echo '<p id="vl-location-help" class="description">' . esc_html__( 'Short links such as maps.app.goo.gl can’t be read here: open the link, then copy the full address from the browser’s address bar.', 'picasso-places' ) . '</p>';
	echo '<p><label for="vl_stops"><strong>' . esc_html__( 'Route stops', 'picasso-places' ) . '</strong> · ' . esc_html__( 'One stop per line: Place name | location | short note. The location takes the same forms as above.', 'picasso-places' ) . '</label></p>';
	echo '<textarea id="vl_stops" name="picasso_places_stops" rows="7" class="vl-geo-lines" data-kind="stop" data-max-lines="' . esc_attr( STOP_LINES ) . '" data-max-chars="' . esc_attr( STOP_CHARS ) . '" data-problems="vl-stops-problems" style="width:100%;max-width:100%" placeholder="Ilulissat | 69.2194, -51.0986 | ' . esc_attr__( 'Starting point', 'picasso-places' ) . '">' . esc_textarea( $stops ) . '</textarea>';
	echo '<div id="vl-stops-problems" class="vl-geo-problems" aria-live="polite">' . geo_problem_list( $stops, 'stop', STOP_LINES ) . '</div>'; // phpcs:ignore WordPress.Security.EscapeOutput -- escaped inside.
	echo '<p>' . esc_html__( 'The map shows the stops in this order, joined by a straight line that is not a road or walking route. Publish only coordinates you are comfortable sharing. Empty coordinates mean no location.', 'picasso-places' ) . '</p>';
	geo_picker( 'post' );
}

/** Whether a classic save of post `$id` came from this plugin's box `$box`, by someone allowed to make it. */
function may_save( $id, $box ) {
	$nonce = 'picasso_places_' . $box . '_nonce';
	return isset( $_POST[ $nonce ] ) && wp_verify_nonce( sanitize_text_field( wp_unslash( $_POST[ $nonce ] ) ), 'picasso_places_' . $box . '_save' )
		&& ! ( defined( 'DOING_AUTOSAVE' ) && DOING_AUTOSAVE ) && ! wp_is_post_revision( $id ) && current_user_can( 'edit_post', $id );
}

function geo_save( $id ) {
	if ( ! may_save( $id, 'geo' ) ) {
		return;
	}
	$key   = 'picasso_places_location_' . $id . '_' . get_current_user_id();
	$typed = isset( $_POST['picasso_places_location'] ) ? sanitize_text_field( vl_geo_unescape( wp_unslash( $_POST['picasso_places_location'] ) ) ) : ''; // phpcs:ignore WordPress.Security.ValidatedSanitizedInput.InputNotSanitized -- sanitized after decoding.
	$point = vl_geo_read( $typed );
	if ( is_array( $point ) ) {
		update_post_meta( $id, '_vl_lat', $point[0] );
		update_post_meta( $id, '_vl_lon', $point[1] );
		delete_transient( $key );
	} elseif ( 'empty' === $point ) {
		delete_post_meta( $id, '_vl_lat' );
		delete_post_meta( $id, '_vl_lon' );
		delete_transient( $key );
	} else {
		set_transient( $key, array( 'text' => $typed, 'why' => $point ), HOUR_IN_SECONDS );
	}
	$stops = clean_lines( isset( $_POST['picasso_places_stops'] ) ? wp_unslash( $_POST['picasso_places_stops'] ) : '', STOP_LINES, STOP_CHARS ); // phpcs:ignore WordPress.Security.ValidatedSanitizedInput.InputNotSanitized -- sanitized line by line.
	if ( '' !== $stops ) {
		update_post_meta( $id, '_vl_stops', wp_slash( $stops ) );
	} else {
		delete_post_meta( $id, '_vl_stops' );
	}
}
add_action( 'save_post_post', __NAMESPACE__ . '\\geo_save' );

function trip_days_box( $post ) {
	wp_nonce_field( 'picasso_places_trip_save', 'picasso_places_trip_nonce' );
	$value = get_post_meta( $post->ID, '_vl_trip_days', true );
	echo '<p>' . esc_html__( 'One place per line:', 'picasso-places' ) . ' <strong>' . esc_html__( 'Day | Place | location | short note | optional post ID', 'picasso-places' ) . '</strong>. ' . esc_html__( 'The location can be coordinates or a map link, as in a story’s Location field.', 'picasso-places' ) . ' ' . esc_html__( 'The map uses these coordinates, not the photos’ EXIF.', 'picasso-places' ) . '</p>';
	echo '<div class="vl-trip-rows hide-if-no-js"></div>';
	echo '<textarea id="vl_trip_days" name="picasso_places_trip_days" rows="11" class="vl-geo-lines" data-kind="day" data-max-lines="' . esc_attr( DAY_LINES ) . '" data-max-chars="' . esc_attr( DAY_CHARS ) . '" data-problems="vl-trip-days-problems" style="width:100%;font-family:monospace" placeholder="1 | Ilulissat | 69.2194, -51.0986 | ' . esc_attr__( 'Arrive and explore', 'picasso-places' ) . '">' . esc_textarea( $value ) . '</textarea>';
	echo '<div id="vl-trip-days-problems" class="vl-geo-problems" aria-live="polite">' . geo_problem_list( $value, 'day', DAY_LINES ) . '</div>'; // phpcs:ignore WordPress.Security.EscapeOutput -- escaped inside.
	echo '<p>' . esc_html__( 'Day numbers set the order of the itinerary. Locations are shown to everyone. Dashed lines show the order of stops, not a route to follow. A post ID links a stop to a published story.', 'picasso-places' ) . '</p>';
	geo_picker( 'trip' );
}

function trip_save( $id ) {
	if ( ! may_save( $id, 'trip' ) ) {
		return;
	}
	$days = clean_lines( isset( $_POST['picasso_places_trip_days'] ) ? wp_unslash( $_POST['picasso_places_trip_days'] ) : '', DAY_LINES, DAY_CHARS ); // phpcs:ignore WordPress.Security.ValidatedSanitizedInput.InputNotSanitized -- sanitized line by line.
	if ( '' !== $days ) {
		update_post_meta( $id, '_vl_trip_days', wp_slash( $days ) );
	} else {
		delete_post_meta( $id, '_vl_trip_days' );
	}
}
add_action( 'save_post_vl_trip', __NAMESPACE__ . '\\trip_save' );

/**
 * Every post matching `$query`, a page at a time.
 * A page that adds nothing stops the walk, so a query that repeats its page cannot run on.
 */
function collect_posts( $query ) {
	$found = array();
	$seen  = array();
	$page  = 1;
	// The id keeps a shared date or title from landing on two pages, or on none.
	if ( ! isset( $query['orderby'] ) ) {
		$query['orderby'] = array(
			'date' => 'DESC',
			'ID'   => 'DESC',
		);
	}
	do {
		$query['posts_per_page'] = 100;
		$query['paged']          = $page;
		$query['no_found_rows']  = true;
		$posts                   = get_posts( $query );
		$added                   = 0;
		foreach ( $posts as $post ) {
			$id = (int) $post->ID;
			if ( isset( $seen[ $id ] ) ) {
				continue;
			}
			$seen[ $id ] = true;
			$found[]     = $post;
			++$added;
		}
		++$page;
	} while ( 100 === count( $posts ) && $added > 0 );
	return $found;
}

function trip_link_box( $post ) {
	wp_nonce_field( 'picasso_places_trip_link_save', 'picasso_places_trip_link_nonce' );
	$selected = absint( get_post_meta( $post->ID, '_vl_trip_id', true ) );
	$trips    = collect_posts(
		array(
			'post_type'   => 'vl_trip',
			'post_status' => array( 'publish', 'draft', 'private' ),
			'orderby'     => array(
				'title' => 'ASC',
				'ID'    => 'ASC',
			),
		)
	);
	$ids = array_map( 'absint', wp_list_pluck( $trips, 'ID' ) );
	if ( $selected && ! in_array( $selected, $ids, true ) ) {
		$current = get_post( $selected );
		if ( $current && 'vl_trip' === $current->post_type ) {
			$trips[] = $current;
		}
	}
	echo '<label for="picasso-places-trip-id">' . esc_html__( 'This story belongs to:', 'picasso-places' ) . '</label>';
	echo '<select id="picasso-places-trip-id" name="picasso_places_trip_id" style="width:100%"><option value="0">' . esc_html__( '— No journey —', 'picasso-places' ) . '</option>';
	foreach ( $trips as $trip ) {
		echo '<option value="' . esc_attr( $trip->ID ) . '" ' . selected( $selected, $trip->ID, false ) . '>' . esc_html( $trip->post_title ) . '</option>';
	}
	echo '</select><p>' . esc_html__( 'Published stories appear on their journey’s page.', 'picasso-places' ) . '</p>';
}

function trip_link_save( $id ) {
	if ( ! may_save( $id, 'trip_link' ) ) {
		return;
	}
	$trip = isset( $_POST['picasso_places_trip_id'] ) ? absint( $_POST['picasso_places_trip_id'] ) : 0;
	if ( $trip && 'vl_trip' === get_post_type( $trip ) ) {
		update_post_meta( $id, '_vl_trip_id', $trip );
	} else {
		delete_post_meta( $id, '_vl_trip_id' );
	}
}
add_action( 'save_post_post', __NAMESPACE__ . '\\trip_link_save' );

/**
 * The stories the itinerary rows can link to, those of this Journey first, and
 * a row for each linked story with a place: its location, or else its first
 * stop, on the day counted from the earliest linked story's date.
 */
function trip_editor_data( $trip_id ) {
	$linked  = $trip_id ? trip_stories( $trip_id ) : array();
	$ids     = wp_list_pluck( $linked, 'ID' );
	$stories = array();
	$rows    = array();
	$first   = null;
	foreach ( $linked as $story ) {
		$stories[] = array( 'id' => $story->ID, 'title' => plain_title( $story ), 'linked' => true );
		$date      = new \DateTimeImmutable( substr( $story->post_date, 0, 10 ) );
		$first     = $first ? $first : $date;
		$lat       = vl_valid_coord( get_post_meta( $story->ID, '_vl_lat', true ), 'lat' );
		$lon       = vl_valid_coord( get_post_meta( $story->ID, '_vl_lon', true ), 'lon' );
		$place     = plain_title( $story );
		if ( null === $lat || null === $lon ) {
			$stops = parse_stops( get_post_meta( $story->ID, '_vl_stops', true ) );
			if ( ! $stops ) {
				continue;
			}
			$place = $stops[0]['name'];
			$lat   = $stops[0]['lat'];
			$lon   = $stops[0]['lon'];
		}
		$rows[] = array(
			'id'       => $story->ID,
			'day'      => min( VL_GEO_DAY_MAX, $first->diff( $date )->days + 1 ),
			'place'    => $place,
			'location' => $lat . ', ' . $lon,
		);
	}
	$others = collect_posts(
		array(
			'post_type'   => 'post',
			'post_status' => 'publish',
			'orderby'     => array(
				'date' => 'DESC',
				'ID'   => 'DESC',
			),
		)
	);
	foreach ( $others as $story ) {
		if ( ! in_array( $story->ID, $ids, true ) ) {
			$stories[] = array( 'id' => $story->ID, 'title' => plain_title( $story ), 'linked' => false );
		}
	}
	return array( 'stories' => $stories, 'linked' => $rows );
}

/** Text the editor's geotag scripts show. */
function geo_admin_text() {
	return array(
		'pickOnMap'     => __( 'Pick on map', 'picasso-places' ),
		'hideMap'       => __( 'Hide map', 'picasso-places' ),
		'loadingMap'    => __( 'Loading map…', 'picasso-places' ),
		'mapFailed'     => __( 'Unable to load the map. Check your connection or content blockers.', 'picasso-places' ),
		'storyLocation' => __( 'Story location', 'picasso-places' ),
		'noLocation'    => __( 'No location set.', 'picasso-places' ),
		/* translators: %s: latitude and longitude, such as 39.86280, -4.02730. */
		'readsAs'       => __( 'Reads as %s', 'picasso-places' ),
		'reasons'       => geo_reasons(),
		'problems'      => __( 'These lines couldn’t be read and are not on the map:', 'picasso-places' ),
		/* translators: 1: line number, 2: the text of the line, 3: why it couldn't be read. */
		'problemLine'   => __( 'Line %1$d: “%2$s” — %3$s', 'picasso-places' ),
		/* translators: 1: most lines the box keeps, 2: most characters it keeps. */
		'limits'        => __( 'This box keeps at most %1$d lines and %2$d characters; the rest is not saved.', 'picasso-places' ),
		/* translators: 1: the location as typed, 2: why it couldn't be read. */
		'locationKept'  => __( 'The location “%1$s” couldn’t be read: %2$s The saved location was kept.', 'picasso-places' ),
		'savedProblems' => __( 'Saved. Some places couldn’t be read and are not on the map.', 'picasso-places' ),
		'searching'     => __( 'Searching…', 'picasso-places' ),
		'noResults'     => __( 'No places found.', 'picasso-places' ),
		'searchFailed'  => __( 'The search failed. Check your connection and try again.', 'picasso-places' ),
		'day'           => __( 'Day', 'picasso-places' ),
		/* translators: %s: day number in a journey. */
		'dayLabel'      => __( 'Day %s', 'picasso-places' ),
		'place'         => __( 'Place', 'picasso-places' ),
		'location'      => __( 'Location', 'picasso-places' ),
		'note'          => __( 'Note', 'picasso-places' ),
		'story'         => __( 'Story', 'picasso-places' ),
		'noStory'       => __( '— No story —', 'picasso-places' ),
		'linkedStories' => __( 'Linked to this journey', 'picasso-places' ),
		'otherStories'  => __( 'Other stories', 'picasso-places' ),
		'order'         => __( 'Order', 'picasso-places' ),
		'moveUp'        => __( 'Move up', 'picasso-places' ),
		'moveDown'      => __( 'Move down', 'picasso-places' ),
		'remove'        => __( 'Remove', 'picasso-places' ),
		'addRow'        => __( 'Add row', 'picasso-places' ),
		'addLinked'     => __( 'Add stops from linked stories', 'picasso-places' ),
		/* translators: %d: number of rows added. */
		'addedLinked'   => __( 'Rows added from linked stories: %d', 'picasso-places' ),
		'noLinked'      => __( 'No linked story with a location is missing from the table.', 'picasso-places' ),
		'editText'      => __( 'Edit as text', 'picasso-places' ),
		'editRows'      => __( 'Edit as rows', 'picasso-places' ),
	);
}

/** The geotag scripts, only on the story and Journey editors. */
function admin_assets( $hook ) {
	$screen = get_current_screen();
	if ( ! in_array( $hook, array( 'post.php', 'post-new.php' ), true ) || ! $screen || ! in_array( $screen->post_type, array( 'post', 'vl_trip' ), true ) ) {
		return;
	}
	$post = get_post();
	$data = array(
		'language' => get_bloginfo( 'language' ),
		'dayMax'   => VL_GEO_DAY_MAX,
		'text'     => geo_admin_text(),
	);
	if ( 'vl_trip' === $screen->post_type ) {
		$data += trip_editor_data( $post ? $post->ID : 0 );
	}
	wp_enqueue_style( 'picasso-places-geo-admin', asset_url( 'css/geo-admin.css' ), array(), VERSION );
	wp_enqueue_script( 'picasso-places-geo', asset_url( 'js/geo.js' ), array(), VERSION, true );
	wp_enqueue_script( 'picasso-places-geo-admin', asset_url( 'js/geo-admin.js' ), array( 'picasso-places-geo' ), VERSION, true );
	wp_add_inline_script( 'picasso-places-geo-admin', 'var vlGeoAdmin = ' . wp_json_encode( $data, JSON_HEX_TAG | JSON_HEX_AMP | JSON_UNESCAPED_UNICODE ) . ';', 'before' );
}
add_action( 'admin_enqueue_scripts', __NAMESPACE__ . '\\admin_assets' );
