<?php
/**
 * A post's location and stops: the editor box, the post fields and the REST API, and the maps.
 *
 * @package VoyageLight
 */

if ( ! defined( 'ABSPATH' ) ) {
	exit;
}

function vl_parse_stops( $text ) {
	$stops = array();
	foreach ( vl_geo_lines( $text, VL_STOP_LINES ) as $line ) {
		$stop = vl_geo_stop_line( $line );
		if ( is_array( $stop ) ) {
			$stop['name'] = sanitize_text_field( $stop['name'] );
			$stop['note'] = sanitize_text_field( $stop['note'] );
			$stops[]      = $stop;
		}
	}
	return $stops;
}

/**
 * The lines of a box of places as the author typed them, without markup and
 * within the box's limits. Lines that can't be read are kept, so the author
 * can correct them; the map skips them.
 */
function vl_clean_lines( $text, $max_lines, $max_chars ) {
	$lines = array();
	$size  = 0;
	foreach ( vl_geo_lines( is_scalar( $text ) ? $text : '', $max_lines ) as $line ) {
		$line  = sanitize_text_field( vl_geo_unescape( $line ) );
		$size += mb_strlen( $line ) + 1;
		if ( $size > $max_chars + 1 ) {
			break;
		}
		$lines[] = $line;
	}
	return rtrim( implode( "\n", $lines ), "\n" );
}

/** Why a location or a line can't be read, keyed by the reasons vl_geo_read() and the line readers give. */
function vl_geo_reasons() {
	return array(
		'empty'      => __( 'There is no location.', 'voyagelight' ),
		'short-link' => __( 'Short links can’t be read without opening them. Open the link, then copy the full address from the browser’s address bar.', 'voyagelight' ),
		'link'       => __( 'This link has no coordinates in it. Open the place on the map, then copy the address again.', 'voyagelight' ),
		'range'      => __( 'Out of range: latitude goes from -90 to 90 and longitude from -180 to 180.', 'voyagelight' ),
		'unreadable' => __( 'This is not a location the theme can read.', 'voyagelight' ),
		'name'       => __( 'The place name is missing.', 'voyagelight' ),
		/* translators: %d: highest day number. */
		'day'        => sprintf( __( 'The day must be a number from 1 to %d.', 'voyagelight' ), VL_GEO_DAY_MAX ),
	);
}

/** A notice naming each line of `$text` that can't be read, or '' when all can. */
function vl_geo_problem_list( $text, $kind, $max_lines ) {
	$problems = vl_geo_problems( $text, $kind, $max_lines );
	if ( ! $problems ) {
		return '';
	}
	$reasons = vl_geo_reasons();
	$out     = '<div class="notice notice-warning inline"><p>' . esc_html__( 'These lines couldn’t be read and are not on the map:', 'voyagelight' ) . '</p><ul>';
	foreach ( $problems as $problem ) {
		/* translators: 1: line number, 2: the text of the line, 3: why it couldn't be read. */
		$out .= '<li>' . esc_html( sprintf( __( 'Line %1$d: “%2$s” — %3$s', 'voyagelight' ), $problem[0], $problem[1], $reasons[ $problem[2] ] ) ) . '</li>';
	}
	return $out . '</ul></div>';
}

/** The Pick on map panel; assets/js/geo-admin.js makes it work. */
function vl_geo_picker( $kind ) {
	$panel = 'vl-geo-panel-' . $kind;
	echo '<div class="vl-geo-picker hide-if-no-js" data-kind="' . esc_attr( $kind ) . '">';
	echo '<p><button type="button" class="button vl-geo-open" aria-expanded="false" aria-controls="' . esc_attr( $panel ) . '">' . esc_html__( 'Pick on map', 'voyagelight' ) . '</button> ';
	echo '<span class="description">' . esc_html__( 'The map loads its tiles from OpenStreetMap, which sees your IP address.', 'voyagelight' ) . '</span></p>';
	echo '<div class="vl-geo-panel" id="' . esc_attr( $panel ) . '" hidden><div class="vl-geo-search">';
	echo '<label for="vl-geo-query-' . esc_attr( $kind ) . '">' . esc_html__( 'Find a place', 'voyagelight' ) . '</label> ';
	echo '<input type="search" id="vl-geo-query-' . esc_attr( $kind ) . '" class="vl-geo-query regular-text" autocomplete="off"> ';
	echo '<button type="button" class="button vl-geo-find">' . esc_html__( 'Search', 'voyagelight' ) . '</button>';
	echo '<p class="description">' . esc_html__( 'Search by OpenStreetMap Nominatim', 'voyagelight' ) . ' · ' . esc_html__( 'Pressing Search sends what you typed to it.', 'voyagelight' ) . '</p>';
	echo '<p class="vl-geo-search-status" aria-live="polite"></p><ul class="vl-geo-results"></ul></div>';
	if ( 'post' === $kind ) {
		echo '<p><label>' . esc_html__( 'A click on the map', 'voyagelight' ) . ' <select class="vl-geo-click">';
		echo '<option value="location">' . esc_html__( 'sets the story’s location', 'voyagelight' ) . '</option>';
		echo '<option value="stop">' . esc_html__( 'adds a route stop', 'voyagelight' ) . '</option></select></label></p>';
	}
	echo '<p class="vl-geo-map-status" aria-live="polite"></p><div class="vl-geo-map"></div>';
	echo '<div class="vl-geo-add" hidden><label>' . esc_html__( 'Place name', 'voyagelight' ) . ' <input type="text" class="vl-geo-add-name"></label>';
	if ( 'trip' === $kind ) {
		echo ' <label>' . esc_html__( 'Day', 'voyagelight' ) . ' <input type="number" class="vl-geo-add-day small-text" min="1" max="' . esc_attr( VL_GEO_DAY_MAX ) . '"></label>';
	}
	echo ' <button type="button" class="button button-primary vl-geo-add-ok">' . esc_html__( 'Add stop', 'voyagelight' ) . '</button>';
	echo ' <button type="button" class="button vl-geo-add-cancel">' . esc_html__( 'Cancel', 'voyagelight' ) . '</button></div>';
	echo '<p class="description">' . ( 'post' === $kind ? esc_html__( 'Click the map to set the location or add a stop. Drag a pin to move it.', 'voyagelight' ) : esc_html__( 'Click the map to add a stop. Drag a pin to move it.', 'voyagelight' ) ) . '</p>';
	echo '</div></div>';
}

function vl_geo_metabox() {
	add_meta_box( 'vl-geo', __( 'VoyageLight · Geotag & Travel Map', 'voyagelight' ), 'vl_geo_fields', 'post', 'normal', 'high' );
}
add_action( 'add_meta_boxes', 'vl_geo_metabox' );

function vl_geo_fields( $post ) {
	wp_nonce_field( 'vl_geo_save', 'vl_geo_nonce' );
	$lat     = vl_valid_coord( get_post_meta( $post->ID, '_vl_lat', true ), 'lat' );
	$lon     = vl_valid_coord( get_post_meta( $post->ID, '_vl_lon', true ), 'lon' );
	$stops   = get_post_meta( $post->ID, '_vl_stops', true );
	$unread  = get_transient( 'vl_location_' . $post->ID . '_' . get_current_user_id() );
	$reasons = vl_geo_reasons();
	$value   = null !== $lat && null !== $lon ? vl_geo_format( $lat, $lon ) : '';
	echo '<p><label for="vl_location"><strong>' . esc_html__( 'Location', 'voyagelight' ) . '</strong></label> · ' . esc_html__( 'Paste coordinates or a map link, such as 39.8628, -4.0273 or 39°51′46″N 4°01′38″W. Links from Google Maps, Apple Maps and OpenStreetMap and geo: addresses work too. Leave it empty for no location.', 'voyagelight' ) . '</p>';
	if ( is_array( $unread ) && isset( $unread['text'], $reasons[ $unread['why'] ] ) ) {
		$value = $unread['text'];
		/* translators: 1: the location as typed, 2: why it couldn't be read. */
		echo '<div class="notice notice-warning inline"><p>' . esc_html( sprintf( __( 'The location “%1$s” couldn’t be read: %2$s The saved location was kept.', 'voyagelight' ), $unread['text'], $reasons[ $unread['why'] ] ) ) . '</p></div>';
	}
	$point = vl_geo_read( $value );
	if ( is_array( $point ) ) {
		/* translators: %s: latitude and longitude, such as 39.86280, -4.02730. */
		$reading = sprintf( __( 'Reads as %s', 'voyagelight' ), sprintf( '%.5F, %.5F', $point[0], $point[1] ) );
	} else {
		$reading = 'empty' === $point ? __( 'No location set.', 'voyagelight' ) : $reasons[ $point ];
	}
	echo '<input type="text" id="vl_location" name="vl_location" class="widefat" value="' . esc_attr( $value ) . '" autocomplete="off" spellcheck="false" aria-describedby="vl-location-reading vl-location-help">';
	echo '<p id="vl-location-reading" class="vl-geo-reading' . ( is_string( $point ) && 'empty' !== $point ? ' is-error' : '' ) . '" aria-live="polite">' . esc_html( $reading ) . '</p>';
	echo '<p id="vl-location-help" class="description">' . esc_html__( 'Short links such as maps.app.goo.gl can’t be read here: open the link, then copy the full address from the browser’s address bar.', 'voyagelight' ) . '</p>';
	echo '<p><label for="vl_stops"><strong>' . esc_html__( 'Route stops', 'voyagelight' ) . '</strong> · ' . esc_html__( 'One stop per line: Place name | location | short note. The location takes the same forms as above.', 'voyagelight' ) . '</label></p>';
	echo '<textarea id="vl_stops" name="vl_stops" rows="7" class="vl-geo-lines" data-kind="stop" data-max-lines="' . esc_attr( VL_STOP_LINES ) . '" data-max-chars="' . esc_attr( VL_STOP_CHARS ) . '" data-problems="vl-stops-problems" style="width:100%;max-width:100%" placeholder="Ilulissat | 69.2194, -51.0986 | ' . esc_attr__( 'Starting point', 'voyagelight' ) . '">' . esc_textarea( $stops ) . '</textarea>';
	echo '<div id="vl-stops-problems" class="vl-geo-problems" aria-live="polite">' . vl_geo_problem_list( $stops, 'stop', VL_STOP_LINES ) . '</div>'; // phpcs:ignore WordPress.Security.EscapeOutput -- escaped inside.
	echo '<p>' . esc_html__( 'The map shows the stops in this order, joined by a straight line that is not a road or walking route. Publish only coordinates you are comfortable sharing. Empty coordinates mean no location.', 'voyagelight' ) . '</p>';
	vl_geo_picker( 'post' );
}

function vl_geo_save( $id ) {
	if ( ! isset( $_POST['vl_geo_nonce'] ) || ! wp_verify_nonce( sanitize_text_field( wp_unslash( $_POST['vl_geo_nonce'] ) ), 'vl_geo_save' )
		|| ( defined( 'DOING_AUTOSAVE' ) && DOING_AUTOSAVE ) || wp_is_post_revision( $id ) || ! current_user_can( 'edit_post', $id ) ) {
		return;
	}
	$key   = 'vl_location_' . $id . '_' . get_current_user_id();
	$typed = isset( $_POST['vl_location'] ) ? sanitize_text_field( vl_geo_unescape( wp_unslash( $_POST['vl_location'] ) ) ) : ''; // phpcs:ignore WordPress.Security.ValidatedSanitizedInput.InputNotSanitized -- sanitized after decoding.
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
	$stops = vl_clean_lines( isset( $_POST['vl_stops'] ) ? wp_unslash( $_POST['vl_stops'] ) : '', VL_STOP_LINES, VL_STOP_CHARS ); // phpcs:ignore WordPress.Security.ValidatedSanitizedInput.InputNotSanitized -- sanitized line by line.
	if ( '' !== $stops ) {
		update_post_meta( $id, '_vl_stops', wp_slash( $stops ) );
	} else {
		delete_post_meta( $id, '_vl_stops' );
	}
}
add_action( 'save_post_post', 'vl_geo_save' );

function vl_meta_auth( $allowed, $meta_key, $object_id ) {
	return current_user_can( 'edit_post', $object_id );
}

function vl_sanitize_lat( $value ) {
	return vl_valid_coord( $value, 'lat' );
}

function vl_sanitize_lon( $value ) {
	return vl_valid_coord( $value, 'lon' );
}

function vl_sanitize_stops( $value ) {
	return vl_clean_lines( $value, VL_STOP_LINES, VL_STOP_CHARS );
}

function vl_sanitize_trip_days( $value ) {
	return vl_clean_lines( $value, VL_DAY_LINES, VL_DAY_CHARS );
}

function vl_sanitize_trip_id( $value ) {
	$id = is_scalar( $value ) ? absint( $value ) : 0;
	return $id && 'vl_trip' === get_post_type( $id ) ? $id : null;
}

/** The geotag fields, also for publishing through the REST API. */
function vl_register_meta() {
	$shared = array(
		'single'        => true,
		'show_in_rest'  => true,
		'auth_callback' => 'vl_meta_auth',
	);
	register_post_meta( 'post', '_vl_lat', $shared + array( 'type' => 'number', 'sanitize_callback' => 'vl_sanitize_lat' ) );
	register_post_meta( 'post', '_vl_lon', $shared + array( 'type' => 'number', 'sanitize_callback' => 'vl_sanitize_lon' ) );
	register_post_meta( 'post', '_vl_stops', $shared + array( 'type' => 'string', 'sanitize_callback' => 'vl_sanitize_stops' ) );
	register_post_meta( 'post', '_vl_trip_id', $shared + array( 'type' => 'integer', 'sanitize_callback' => 'vl_sanitize_trip_id' ) );
	register_post_meta( 'vl_trip', '_vl_trip_days', $shared + array( 'type' => 'string', 'sanitize_callback' => 'vl_sanitize_trip_days' ) );
}
add_action( 'init', 'vl_register_meta' );

/** A geotag field that sanitizes to nothing, such as a latitude out of range, is removed rather than stored empty. */
function vl_meta_drop_empty( $check, $object_id, $meta_key, $meta_value ) {
	if ( in_array( $meta_key, array( '_vl_lat', '_vl_lon', '_vl_stops', '_vl_trip_id', '_vl_trip_days' ), true ) && ( null === $meta_value || '' === $meta_value ) ) {
		delete_metadata( 'post', $object_id, $meta_key );
		return true;
	}
	return $check;
}
add_filter( 'add_post_metadata', 'vl_meta_drop_empty', 10, 4 );
add_filter( 'update_post_metadata', 'vl_meta_drop_empty', 10, 4 );

/**
 * REST shows a missing coordinate as 0. Sending 0, 0 back with no stop line
 * means no location. A stop line keeps a place at 0, 0.
 */
function vl_rest_zero_location( $prepared, $request ) {
	$meta = $request['meta'];
	if ( ! is_array( $meta ) || ! isset( $meta['_vl_lat'], $meta['_vl_lon'] ) || 0.0 !== (float) $meta['_vl_lat'] || 0.0 !== (float) $meta['_vl_lon'] ) {
		return $prepared;
	}
	$post_id = ! empty( $request['id'] ) ? (int) $request['id'] : 0;
	if ( array_key_exists( '_vl_stops', $meta ) ) {
		$raw   = $meta['_vl_stops'];
		$stops = is_scalar( $raw ) ? trim( (string) $raw ) : '';
	} else {
		$stops = $post_id ? trim( (string) get_post_meta( $post_id, '_vl_stops', true ) ) : '';
	}
	if ( '' !== $stops ) {
		return $prepared;
	}
	foreach ( array( '_vl_lat', '_vl_lon' ) as $key ) {
		if ( $post_id && metadata_exists( 'post', $post_id, $key ) ) {
			$meta[ $key ] = null;
		} else {
			unset( $meta[ $key ] );
		}
	}
	$request['meta'] = $meta;
	return $prepared;
}
add_filter( 'rest_pre_insert_post', 'vl_rest_zero_location', 10, 2 );

/**
 * The REST API leaves out the places of a password-protected story or Journey,
 * as the page does, unless the reader may edit it or sends its password.
 */
function vl_rest_hide_protected_places( $response, $post, $request ) {
	if ( '' === $post->post_password || current_user_can( 'edit_post', $post->ID ) || hash_equals( $post->post_password, (string) $request['password'] ) ) {
		return $response;
	}
	$data = $response->get_data();
	if ( isset( $data['meta'] ) && is_array( $data['meta'] ) ) {
		$data['meta'] = array_diff_key( $data['meta'], array_flip( array( '_vl_lat', '_vl_lon', '_vl_stops', '_vl_trip_days' ) ) );
		$response->set_data( $data );
	}
	return $response;
}
add_filter( 'rest_prepare_post', 'vl_rest_hide_protected_places', 10, 3 );
add_filter( 'rest_prepare_vl_trip', 'vl_rest_hide_protected_places', 10, 3 );

/** The places on post `$id`'s map; none while the post's password hasn't been entered. */
function vl_post_points( $id ) {
	if ( post_password_required( $id ) ) {
		return array();
	}
	$points = vl_parse_stops( get_post_meta( $id, '_vl_stops', true ) );
	if ( $points ) {
		return $points;
	}
	$lat = vl_valid_coord( get_post_meta( $id, '_vl_lat', true ), 'lat' );
	$lon = vl_valid_coord( get_post_meta( $id, '_vl_lon', true ), 'lon' );
	return null !== $lat && null !== $lon ? array( array( 'name' => vl_plain_title( $id ), 'lat' => $lat, 'lon' => $lon, 'note' => '' ) ) : array();
}

function vl_map_data( $scope = 'all', $post_id = 0 ) {
	if ( 'post' === $scope ) {
		return vl_post_points( $post_id );
	}
	// Only published posts without a password are shown on the atlas.
	// Whole posts, not IDs, so their fields and meta come in one query each.
	$points = array();
	foreach (
		vl_collect_posts(
			array(
				'post_type'    => 'post',
				'post_status'  => 'publish',
				'has_password' => false,
				'meta_query'   => array( array( 'key' => '_vl_lat', 'compare' => 'EXISTS' ) ),
			)
		) as $post
	) {
		$lat = vl_valid_coord( get_post_meta( $post->ID, '_vl_lat', true ), 'lat' );
		$lon = vl_valid_coord( get_post_meta( $post->ID, '_vl_lon', true ), 'lon' );
		if ( null !== $lat && null !== $lon ) {
			$points[] = array( 'name' => vl_plain_title( $post ), 'lat' => $lat, 'lon' => $lon, 'note' => '', 'url' => get_permalink( $post ) );
		}
	}
	return $points;
}

/**
 * Every post matching `$query`, a page at a time.
 * A page that adds nothing stops the walk, so a query that repeats its page cannot run on.
 */
function vl_collect_posts( $query ) {
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

/**
 * A map of `$points` that loads only when the reader asks, with its numbered stops.
 * `$show_heading` false leaves out the map's own heading, for a section that has one;
 * `$heading` still names the map for screen readers.
 */
function vl_render_map( $points, $heading, $route = false, $show_heading = true ) {
	if ( ! $points ) {
		return '';
	}
	$intro = '';
	if ( $show_heading ) {
		$intro = '<div class="vl-map-heading"><div><span class="eyebrow">' . esc_html__( 'Geographic context', 'voyagelight' ) . '</span>'
			. '<h2>' . esc_html( $heading ) . '</h2>'
			. '<p>' . esc_html__( 'See where the places are in relation to each other, and select a pin to read its note. Lines show the order of stops, not the actual route.', 'voyagelight' ) . '</p></div></div>';
	}
	$json  = wp_json_encode( array( 'points' => array_values( $points ), 'route' => $route ), JSON_HEX_TAG | JSON_HEX_APOS | JSON_HEX_AMP | JSON_HEX_QUOT );
	$count = count( $points );
	$stops = '';
	foreach ( $points as $stop ) {
		$stops .= '<li><strong>' . esc_html( $stop['name'] ) . '</strong>';
		if ( ! empty( $stop['note'] ) ) {
			$stops .= ' · ' . esc_html( $stop['note'] );
		}
		if ( ! empty( $stop['url'] ) ) {
			$stops .= ' · <a href="' . esc_url( $stop['url'] ) . '">' . esc_html__( 'Read story ↗', 'voyagelight' ) . '</a>';
		}
		$stops .= '</li>';
	}
	return '<section class="vl-map-section">' . $intro
		. '<div class="vl-map" role="region" aria-label="' . esc_attr( $heading ) . '" data-map="' . esc_attr( $json ) . '"><div class="vl-map-placeholder">' . vl_pin_icon()
		/* translators: %d: number of places on the map. */
		. '<p>' . esc_html( sprintf( _n( 'Interactive map · %d place', 'Interactive map · %d places', $count, 'voyagelight' ), $count ) ) . '</p>'
		. '<button class="vl-map-load" type="button">' . esc_html__( 'Load map', 'voyagelight' ) . '</button>'
		. '<small>' . esc_html__( 'Uses map tiles from OpenStreetMap. Loading shares your IP address with it.', 'voyagelight' ) . '</small></div></div>'
		. '<ol class="vl-map-stops">' . $stops . '</ol></section>';
}

add_shortcode(
	'voyagelight_map',
	function ( $atts ) {
		$atts  = shortcode_atts( array( 'scope' => 'post' ), $atts );
		$scope = 'all' === $atts['scope'] ? 'all' : 'post';
		$title = 'all' === $scope ? __( 'Travel atlas', 'voyagelight' ) : __( 'Places in this story', 'voyagelight' );
		return vl_render_map( vl_map_data( $scope, get_the_ID() ), $title, 'post' === $scope );
	}
);
add_shortcode(
	'voyagelight_atlas',
	function () {
		return vl_render_map( vl_map_data(), __( 'Travel atlas', 'voyagelight' ), false );
	}
);
