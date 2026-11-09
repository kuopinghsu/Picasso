<?php
/**
 * Journeys: the post type, its editor boxes, the editor's location scripts and the Journey cards.
 *
 * @package VoyageLight
 */

if ( ! defined( 'ABSPATH' ) ) {
	exit;
}

/** Journeys: a trip with day-by-day stops, travel stories and a map. */
function vl_register_trips() {
	register_post_type(
		'vl_trip',
		array(
			'labels'       => array(
				'name'          => __( 'Journeys', 'voyagelight' ),
				'singular_name' => __( 'Journey', 'voyagelight' ),
				'add_new_item'  => __( 'Add Journey', 'voyagelight' ),
				'edit_item'     => __( 'Edit Journey', 'voyagelight' ),
			),
			'public'       => true,
			'has_archive'  => true,
			'show_in_rest' => true,
			'rest_base'    => 'vl_trip',
			'menu_icon'    => 'dashicons-location-alt',
			'rewrite'      => array( 'slug' => vl_permalink_base( 'trip' ) ),
			'supports'     => array( 'title', 'editor', 'excerpt', 'thumbnail', 'revisions', 'custom-fields' ),
		)
	);
}
add_action( 'init', 'vl_register_trips' );

function vl_trip_parse_days( $raw ) {
	$rows = array();
	foreach ( vl_geo_lines( $raw, VL_DAY_LINES ) as $line ) {
		$row = vl_geo_day_line( $line );
		if ( is_array( $row ) ) {
			$row['name'] = sanitize_text_field( $row['name'] );
			$row['note'] = sanitize_text_field( $row['note'] );
			$rows[]      = $row;
		}
	}
	// usort isn't stable before PHP 8; the line order breaks ties within a Day.
	$order = array_keys( $rows );
	array_multisort( array_column( $rows, 'day' ), SORT_ASC, $order, SORT_ASC, $rows );
	return $rows;
}

function vl_trip_editor_boxes() {
	add_meta_box( 'vl-trip-days', __( 'Journey itinerary & GeoTags', 'voyagelight' ), 'vl_trip_days_box', 'vl_trip', 'normal', 'high' );
	add_meta_box( 'vl-trip-link', __( 'Related journey', 'voyagelight' ), 'vl_trip_link_box', 'post', 'side', 'high' );
}
add_action( 'add_meta_boxes', 'vl_trip_editor_boxes' );

function vl_trip_days_box( $post ) {
	wp_nonce_field( 'vl_trip_save', 'vl_trip_nonce' );
	$value = get_post_meta( $post->ID, '_vl_trip_days', true );
	echo '<p>' . esc_html__( 'One place per line:', 'voyagelight' ) . ' <strong>' . esc_html__( 'Day | Place | location | short note | optional post ID', 'voyagelight' ) . '</strong>. ' . esc_html__( 'The location can be coordinates or a map link, as in a story’s Location field.', 'voyagelight' ) . ' ' . esc_html__( 'The map uses these coordinates, not the photos’ EXIF.', 'voyagelight' ) . '</p>';
	echo '<div class="vl-trip-rows hide-if-no-js"></div>';
	echo '<textarea id="vl_trip_days" name="vl_trip_days" rows="11" class="vl-geo-lines" data-kind="day" data-max-lines="' . esc_attr( VL_DAY_LINES ) . '" data-max-chars="' . esc_attr( VL_DAY_CHARS ) . '" data-problems="vl-trip-days-problems" style="width:100%;font-family:monospace" placeholder="1 | Ilulissat | 69.2194, -51.0986 | ' . esc_attr__( 'Arrive and explore', 'voyagelight' ) . '">' . esc_textarea( $value ) . '</textarea>';
	echo '<div id="vl-trip-days-problems" class="vl-geo-problems" aria-live="polite">' . vl_geo_problem_list( $value, 'day', VL_DAY_LINES ) . '</div>'; // phpcs:ignore WordPress.Security.EscapeOutput -- escaped inside.
	echo '<p>' . esc_html__( 'Day numbers set the order of the itinerary. Locations are shown to everyone. Dashed lines show the order of stops, not a route to follow. A post ID links a stop to a published story.', 'voyagelight' ) . '</p>';
	vl_geo_picker( 'trip' );
}

/** A title as plain text for the editor's scripts. */
function vl_plain_title( $post ) {
	return html_entity_decode( wp_strip_all_tags( get_the_title( $post ) ), ENT_QUOTES, 'UTF-8' );
}

/**
 * The stories the itinerary rows can link to, those of this Journey first, and
 * a row for each linked story with a place: its location, or else its first
 * stop, on the day counted from the earliest linked story's date.
 */
function vl_trip_editor_data( $trip_id ) {
	$linked  = $trip_id ? vl_trip_stories( $trip_id ) : array();
	$ids     = wp_list_pluck( $linked, 'ID' );
	$stories = array();
	$rows    = array();
	$first   = null;
	foreach ( $linked as $story ) {
		$stories[] = array( 'id' => $story->ID, 'title' => vl_plain_title( $story ), 'linked' => true );
		$date      = new DateTimeImmutable( substr( $story->post_date, 0, 10 ) );
		$first     = $first ? $first : $date;
		$lat       = vl_valid_coord( get_post_meta( $story->ID, '_vl_lat', true ), 'lat' );
		$lon       = vl_valid_coord( get_post_meta( $story->ID, '_vl_lon', true ), 'lon' );
		$place     = vl_plain_title( $story );
		if ( null === $lat || null === $lon ) {
			$stops = vl_parse_stops( get_post_meta( $story->ID, '_vl_stops', true ) );
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
			'location' => vl_geo_format( $lat, $lon ),
		);
	}
	// Only each story's id and title: whole posts, with their content, fields and terms, would load every story on the site.
	global $wpdb;
	$others = $wpdb->get_results( $wpdb->prepare( "SELECT ID, post_title FROM {$wpdb->posts} WHERE post_type = %s AND post_status = %s ORDER BY post_date DESC, ID DESC", 'post', 'publish' ) ); // phpcs:ignore WordPress.DB.DirectDatabaseQuery -- a read of two columns, fresh each time the editor opens.
	$skip   = array_flip( array_map( 'intval', $ids ) );
	foreach ( $others as $story ) {
		$id = (int) $story->ID;
		if ( ! isset( $skip[ $id ] ) ) {
			$stories[] = array( 'id' => $id, 'title' => html_entity_decode( wp_strip_all_tags( $story->post_title ), ENT_QUOTES, 'UTF-8' ), 'linked' => false );
		}
	}
	return array( 'stories' => $stories, 'linked' => $rows );
}

/** Text the editor's geotag scripts show. */
function vl_geo_admin_text() {
	return array(
		'pickOnMap'     => __( 'Pick on map', 'voyagelight' ),
		'hideMap'       => __( 'Hide map', 'voyagelight' ),
		'loadingMap'    => __( 'Loading map…', 'voyagelight' ),
		'mapFailed'     => __( 'Unable to load the map. Check your connection or content blockers.', 'voyagelight' ),
		'storyLocation' => __( 'Story location', 'voyagelight' ),
		'noLocation'    => __( 'No location set.', 'voyagelight' ),
		/* translators: %s: latitude and longitude, such as 39.86280, -4.02730. */
		'readsAs'       => __( 'Reads as %s', 'voyagelight' ),
		'reasons'       => vl_geo_reasons(),
		'problems'      => __( 'These lines couldn’t be read and are not on the map:', 'voyagelight' ),
		/* translators: 1: line number, 2: the text of the line, 3: why it couldn't be read. */
		'problemLine'   => __( 'Line %1$d: “%2$s” — %3$s', 'voyagelight' ),
		/* translators: 1: most lines the box keeps, 2: most characters it keeps. */
		'limits'        => __( 'This box keeps at most %1$d lines and %2$d characters; the rest is not saved.', 'voyagelight' ),
		/* translators: 1: the location as typed, 2: why it couldn't be read. */
		'locationKept'  => __( 'The location “%1$s” couldn’t be read: %2$s The saved location was kept.', 'voyagelight' ),
		'savedProblems' => __( 'Saved. Some places couldn’t be read and are not on the map.', 'voyagelight' ),
		'searching'     => __( 'Searching…', 'voyagelight' ),
		'noResults'     => __( 'No places found.', 'voyagelight' ),
		'searchFailed'  => __( 'The search failed. Check your connection and try again.', 'voyagelight' ),
		'day'           => __( 'Day', 'voyagelight' ),
		/* translators: %s: day number in a journey. */
		'dayLabel'      => __( 'Day %s', 'voyagelight' ),
		'place'         => __( 'Place', 'voyagelight' ),
		'location'      => __( 'Location', 'voyagelight' ),
		'note'          => __( 'Note', 'voyagelight' ),
		'story'         => __( 'Story', 'voyagelight' ),
		'noStory'       => __( '— No story —', 'voyagelight' ),
		'linkedStories' => __( 'Linked to this journey', 'voyagelight' ),
		'otherStories'  => __( 'Other stories', 'voyagelight' ),
		'order'         => __( 'Order', 'voyagelight' ),
		'moveUp'        => __( 'Move up', 'voyagelight' ),
		'moveDown'      => __( 'Move down', 'voyagelight' ),
		'remove'        => __( 'Remove', 'voyagelight' ),
		'addRow'        => __( 'Add row', 'voyagelight' ),
		'addLinked'     => __( 'Add stops from linked stories', 'voyagelight' ),
		/* translators: %d: number of rows added. */
		'addedLinked'   => __( 'Rows added from linked stories: %d', 'voyagelight' ),
		'noLinked'      => __( 'No linked story with a location is missing from the table.', 'voyagelight' ),
		'editText'      => __( 'Edit as text', 'voyagelight' ),
		'editRows'      => __( 'Edit as rows', 'voyagelight' ),
	);
}

/** The geotag scripts, only on the story and Journey editors. */
function vl_admin_assets( $hook ) {
	$screen = get_current_screen();
	if ( ! in_array( $hook, array( 'post.php', 'post-new.php' ), true ) || ! $screen || ! in_array( $screen->post_type, array( 'post', 'vl_trip' ), true ) ) {
		return;
	}
	$uri  = get_template_directory_uri();
	$post = get_post();
	$data = array(
		'language' => get_bloginfo( 'language' ),
		'dayMax'   => VL_GEO_DAY_MAX,
		'leaflet'  => $uri . '/assets/vendor/leaflet/',
		'text'     => vl_geo_admin_text(),
	);
	if ( 'vl_trip' === $screen->post_type ) {
		$data += vl_trip_editor_data( $post ? $post->ID : 0 );
	}
	wp_enqueue_style( 'vl-geo-admin', $uri . '/assets/css/geo-admin.css', array(), VL_VERSION );
	wp_enqueue_script( 'vl-geo', $uri . '/assets/js/geo.js', array(), VL_VERSION, true );
	wp_enqueue_script( 'vl-geo-admin', $uri . '/assets/js/geo-admin.js', array( 'vl-geo' ), VL_VERSION, true );
	wp_add_inline_script( 'vl-geo-admin', 'var vlGeoAdmin = ' . wp_json_encode( $data, JSON_HEX_TAG | JSON_HEX_AMP | JSON_UNESCAPED_UNICODE ) . ';', 'before' );
}
add_action( 'admin_enqueue_scripts', 'vl_admin_assets' );

function vl_trip_link_box( $post ) {
	wp_nonce_field( 'vl_trip_link_save', 'vl_trip_link_nonce' );
	$selected = absint( get_post_meta( $post->ID, '_vl_trip_id', true ) );
	$trips    = vl_collect_posts(
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
	echo '<label for="vl_trip_id">' . esc_html__( 'This story belongs to:', 'voyagelight' ) . '</label>';
	echo '<select id="vl_trip_id" name="vl_trip_id" style="width:100%"><option value="0">' . esc_html__( '— No journey —', 'voyagelight' ) . '</option>';
	foreach ( $trips as $trip ) {
		echo '<option value="' . esc_attr( $trip->ID ) . '" ' . selected( $selected, $trip->ID, false ) . '>' . esc_html( $trip->post_title ) . '</option>';
	}
	echo '</select><p>' . esc_html__( 'Published stories appear on their journey’s page.', 'voyagelight' ) . '</p>';
}

function vl_trip_save( $id ) {
	if ( ! isset( $_POST['vl_trip_nonce'] ) || ! wp_verify_nonce( sanitize_text_field( wp_unslash( $_POST['vl_trip_nonce'] ) ), 'vl_trip_save' )
		|| ( defined( 'DOING_AUTOSAVE' ) && DOING_AUTOSAVE ) || wp_is_post_revision( $id ) || ! current_user_can( 'edit_post', $id ) ) {
		return;
	}
	$days = vl_clean_lines( isset( $_POST['vl_trip_days'] ) ? wp_unslash( $_POST['vl_trip_days'] ) : '', VL_DAY_LINES, VL_DAY_CHARS ); // phpcs:ignore WordPress.Security.ValidatedSanitizedInput.InputNotSanitized -- sanitized line by line.
	if ( '' !== $days ) {
		update_post_meta( $id, '_vl_trip_days', wp_slash( $days ) );
	} else {
		delete_post_meta( $id, '_vl_trip_days' );
	}
}
add_action( 'save_post_vl_trip', 'vl_trip_save' );

function vl_trip_link_save( $id ) {
	if ( ! isset( $_POST['vl_trip_link_nonce'] ) || ! wp_verify_nonce( sanitize_text_field( wp_unslash( $_POST['vl_trip_link_nonce'] ) ), 'vl_trip_link_save' )
		|| ( defined( 'DOING_AUTOSAVE' ) && DOING_AUTOSAVE ) || wp_is_post_revision( $id ) || ! current_user_can( 'edit_post', $id ) ) {
		return;
	}
	$trip = isset( $_POST['vl_trip_id'] ) ? absint( $_POST['vl_trip_id'] ) : 0;
	if ( $trip && 'vl_trip' === get_post_type( $trip ) ) {
		update_post_meta( $id, '_vl_trip_id', $trip );
	} else {
		delete_post_meta( $id, '_vl_trip_id' );
	}
}
add_action( 'save_post_post', 'vl_trip_link_save' );

function vl_trip_stories( $id ) {
	return get_posts(
		array(
			'post_type'      => 'post',
			'post_status'    => 'publish',
			'posts_per_page' => -1,
			'meta_key'       => '_vl_trip_id',
			'meta_value'     => $id,
			'orderby'        => array(
				'date' => 'ASC',
				'ID'   => 'ASC',
			),
		)
	);
}

function vl_day_label( $day ) {
	/* translators: %s: day number in a journey. */
	return sprintf( __( 'Day %s', 'voyagelight' ), number_format_i18n( $day ) );
}

/**
 * Stops for the Journey page's map: the place, the day and, for a published
 * story, its address. The post id stays on the server.
 */
function vl_trip_explorer_stops( $days ) {
	$stops = array();
	foreach ( $days as $row ) {
		$stop = array(
			'name' => $row['name'],
			'lat'  => $row['lat'],
			'lon'  => $row['lon'],
			'note' => $row['note'],
			'day'  => $row['day'],
		);
		if ( ! empty( $row['post_id'] ) && 'post' === get_post_type( $row['post_id'] ) && 'publish' === get_post_status( $row['post_id'] ) ) {
			$stop['url'] = get_permalink( $row['post_id'] );
		}
		$stops[] = $stop;
	}
	return $stops;
}

function vl_trip_points( $trip_id ) {
	$points = array();
	foreach ( vl_trip_parse_days( get_post_meta( $trip_id, '_vl_trip_days', true ) ) as $row ) {
		$point = array(
			'name' => $row['name'],
			'lat'  => $row['lat'],
			'lon'  => $row['lon'],
			'note' => vl_day_label( $row['day'] ) . ( $row['note'] ? ' · ' . $row['note'] : '' ),
			'day'  => $row['day'],
		);
		if ( $row['post_id'] && 'publish' === get_post_status( $row['post_id'] ) && 'post' === get_post_type( $row['post_id'] ) ) {
			$point['url'] = get_permalink( $row['post_id'] );
		}
		$points[] = $point;
	}
	return $points;
}

function vl_trip_card( $id ) {
	$stops = count( vl_trip_points( $id ) );
	echo '<article class="story-card trip-card"><div class="story-visual">';
	if ( has_post_thumbnail( $id ) ) {
		echo get_the_post_thumbnail( $id, 'vl-card', array( 'loading' => 'lazy', 'alt' => '' ) );
	} else {
		echo '<img src="' . esc_url( vl_card_art( $id ) ) . '" alt="" loading="lazy">';
	}
	echo '<span class="story-badge">' . esc_html__( 'Journey', 'voyagelight' ) . '</span></div>';
	/* translators: %d: number of stops in a journey. */
	echo '<div class="story-copy"><span class="eyebrow">' . esc_html( sprintf( _n( '%d stop', '%d stops', $stops, 'voyagelight' ), $stops ) ) . '</span>';
	echo '<h3><a href="' . esc_url( get_permalink( $id ) ) . '">' . esc_html( get_the_title( $id ) ) . '</a></h3>';
	echo '<p>' . esc_html( vl_card_summary( $id ) ) . '</p></div></article>';
}
