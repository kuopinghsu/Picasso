<?php
/**
 * Maps, itineraries and Journey links after a post's content, for any theme.
 * assets/js/map.js, shared with the VoyageLight theme, loads a map from the
 * .vl-map element's data-map when its .vl-map-load button is pressed.
 *
 * @package PicassoPlaces
 */

namespace PicassoPlaces;

if ( ! defined( 'ABSPATH' ) ) {
	exit;
}

/** Text map.js shows, in the site's language. */
function script_text() {
	return array(
		'loadingMap' => __( 'Loading map…', 'picasso-places' ),
		'retryMap'   => __( 'Retry loading the map', 'picasso-places' ),
		'mapFailed'  => __( 'Unable to load the map. Check your connection or content blockers.', 'picasso-places' ),
		'place'      => __( 'Place', 'picasso-places' ),
		'readStory'  => __( 'Read story ↗', 'picasso-places' ),
	);
}

/** The places on post or Journey `$id`'s map. */
function map_points( $id ) {
	return 'vl_trip' === get_post_type( $id ) ? trip_points( $id ) : post_points( $id );
}

/** The styles on single posts and Journeys only, where the plugin adds its parts; the map script only when there is a map. */
function front_assets() {
	if ( is_admin() || ! is_singular( array( 'post', 'vl_trip' ) ) ) {
		return;
	}
	wp_enqueue_style( 'picasso-places', asset_url( 'css/places.css' ), array(), VERSION );
	if ( post_password_required() || ! map_points( get_queried_object_id() ) ) {
		return;
	}
	wp_enqueue_script( 'picasso-places-map', asset_url( 'js/map.js' ), array(), VERSION, true );
	wp_localize_script( 'picasso-places-map', 'vlText', script_text() + array( 'leaflet' => asset_url( 'vendor/leaflet/' ) ) );
}
add_action( 'wp_enqueue_scripts', __NAMESPACE__ . '\\front_assets' );

/**
 * Whether `the_content` is drawing the main post of a single post or Journey
 * page, rather than a feed, an excerpt, the REST API or another post on the page.
 */
function is_main_content() {
	return ! is_admin() && ! is_feed() && ! is_embed() && ! ( defined( 'REST_REQUEST' ) && REST_REQUEST )
		&& is_singular( array( 'post', 'vl_trip' ) ) && in_the_loop() && is_main_query()
		&& get_the_ID() === get_queried_object_id()
		&& ! doing_filter( 'get_the_excerpt' ) && ! doing_filter( 'wp_trim_excerpt' ) && ! post_password_required();
}

/** A map that loads only when the reader asks, with a note on what loading fetches. */
function render_map( $points, $route, $label, $name ) {
	$json = wp_json_encode( array( 'points' => array_values( $points ), 'route' => $route ), JSON_HEX_TAG | JSON_HEX_APOS | JSON_HEX_AMP | JSON_HEX_QUOT );
	return '<div class="picasso-places-frame vl-map" role="region" aria-label="' . esc_attr( $name ) . '" data-map="' . esc_attr( $json ) . '"><div class="picasso-places-placeholder">'
		. '<p>' . esc_html( $label ) . '</p>'
		. '<button type="button" class="picasso-places-load vl-map-load wp-element-button">' . esc_html__( 'Load map', 'picasso-places' ) . '</button>'
		. '<small>' . esc_html__( 'Uses map tiles from OpenStreetMap. Loading shares your IP address with it.', 'picasso-places' ) . '</small>'
		. '</div></div>';
}

function stop_item( $name, $note, $url ) {
	$out = '<strong>' . esc_html( $name ) . '</strong>';
	if ( '' !== $note ) {
		$out .= ' · ' . esc_html( $note );
	}
	if ( $url ) {
		$out .= ' · <a href="' . esc_url( $url ) . '">' . esc_html__( 'Read story ↗', 'picasso-places' ) . '</a>';
	}
	return $out;
}

/** A story's map and, when it has more to say than one pin, its numbered stops. */
function story_section( $id ) {
	$points = post_points( $id );
	if ( ! $points ) {
		return '';
	}
	$count   = count( $points );
	$heading = __( 'Places in this story', 'picasso-places' );
	/* translators: %d: number of places on the map. */
	$label = sprintf( _n( 'Interactive map · %d place', 'Interactive map · %d places', $count, 'picasso-places' ), $count );
	$out   = '<section class="picasso-places-section picasso-places-story"><h2>' . esc_html( $heading ) . '</h2>' . render_map( $points, true, $label, $heading );
	if ( $count > 1 || '' !== $points[0]['note'] ) {
		$out .= '<ol class="picasso-places-stops">';
		foreach ( $points as $stop ) {
			$out .= '<li>' . stop_item( $stop['name'], $stop['note'], '' ) . '</li>';
		}
		$out .= '</ol>';
	}
	return $out . '</section>';
}

/** A Journey's map, its stops day by day, numbered in the order the map joins them, and its stories. */
function journey_section( $id ) {
	$days    = trip_parse_days( get_post_meta( $id, '_vl_trip_days', true ) );
	$stories = trip_stories( $id );
	if ( ! $days && ! $stories ) {
		return '';
	}
	$out = '<section class="picasso-places-section picasso-places-journey">';
	if ( $days ) {
		$heading = __( 'Journey map', 'picasso-places' );
		$out    .= '<h2>' . esc_html( $heading ) . '</h2>' . render_map( trip_points( $id ), true, __( 'Interactive journey map', 'picasso-places' ), $heading );
		$out    .= '<h2>' . esc_html__( 'Itinerary', 'picasso-places' ) . '</h2>';
		$current = 0;
		$number  = 0;
		foreach ( $days as $row ) {
			if ( $current !== $row['day'] ) {
				if ( $current ) {
					$out .= '</ol></div>';
				}
				$current = $row['day'];
				$out    .= '<div class="picasso-places-day"><h3>' . esc_html( day_label( $current ) ) . '</h3><ol class="picasso-places-stops" start="' . ( $number + 1 ) . '">';
			}
			++$number;
			$out .= '<li>' . stop_item( $row['name'], $row['note'], story_url( $row['post_id'] ) ) . '</li>';
		}
		$out .= '</ol></div>';
	}
	if ( $stories ) {
		$out .= '<h2>' . esc_html__( 'Stories from this journey', 'picasso-places' ) . '</h2><ul class="picasso-places-stories">';
		foreach ( $stories as $story ) {
			$out .= '<li><a href="' . esc_url( get_permalink( $story ) ) . '">' . esc_html( plain_title( $story ) ) . '</a></li>';
		}
		$out .= '</ul>';
	}
	return $out . '</section>';
}

/** "Part of the journey …", when a story belongs to a published Journey. */
function journey_link( $id ) {
	$trip = absint( get_post_meta( $id, '_vl_trip_id', true ) );
	if ( ! $trip || 'vl_trip' !== get_post_type( $trip ) || 'publish' !== get_post_status( $trip ) ) {
		return '';
	}
	$link = '<a href="' . esc_url( get_permalink( $trip ) ) . '">' . esc_html( plain_title( $trip ) ) . '</a>';
	/* translators: %s: the journey's name, linked to its page. */
	return '<p class="picasso-places-journey-link">' . sprintf( esc_html__( 'Part of the journey %s', 'picasso-places' ), $link ) . '</p>';
}

function add_to_content( $content ) {
	if ( ! is_main_content() ) {
		return $content;
	}
	$id = get_the_ID();
	if ( 'vl_trip' === get_post_type( $id ) ) {
		return $content . journey_section( $id );
	}
	return $content . journey_link( $id ) . story_section( $id );
}
add_filter( 'the_content', __NAMESPACE__ . '\\add_to_content', 15 );

/**
 * How an uploaded photo was taken, from what WordPress read of its EXIF.
 * Picasso sends these only with Keep camera details. '' when the photo has none.
 */
function camera_line( $attachment_id ) {
	$meta  = wp_get_attachment_metadata( $attachment_id );
	$exif  = ( is_array( $meta ) && isset( $meta['image_meta'] ) && is_array( $meta['image_meta'] ) ) ? $meta['image_meta'] : array();
	$parts = array();
	if ( ! empty( $exif['camera'] ) ) {
		$parts[] = $exif['camera'];
	}
	if ( ! empty( $exif['focal_length'] ) && (float) $exif['focal_length'] > 0 ) {
		$parts[] = round( (float) $exif['focal_length'] ) . ' mm';
	}
	if ( ! empty( $exif['aperture'] ) && (float) $exif['aperture'] > 0 ) {
		$parts[] = 'f/' . round( (float) $exif['aperture'], 1 );
	}
	$seconds = isset( $exif['shutter_speed'] ) ? (float) $exif['shutter_speed'] : 0;
	if ( $seconds > 0 ) {
		$parts[] = $seconds < 0.5 ? '1/' . round( 1 / $seconds ) . ' s' : round( $seconds, 1 ) . ' s';
	}
	if ( ! empty( $exif['iso'] ) && absint( $exif['iso'] ) > 0 ) {
		$parts[] = 'ISO ' . absint( $exif['iso'] );
	}
	return implode( ' · ', $parts );
}

/** Marks images inside a gallery, which keep to their own captions. */
function mark_gallery_images( $block, $source, $parent = null ) {
	if ( 'core/image' === $block['blockName'] && $parent instanceof \WP_Block && 'core/gallery' === $parent->name ) {
		$block['attrs']['picassoPlacesInGallery'] = true;
	}
	return $block;
}
add_filter( 'render_block_data', __NAMESPACE__ . '\\mark_gallery_images', 10, 3 );

/** Adds the camera details to an Image block's caption, making one if it has none. */
function image_camera( $content, $block ) {
	if ( empty( $block['attrs']['id'] ) || ! empty( $block['attrs']['picassoPlacesInGallery'] ) ) {
		return $content;
	}
	$line = camera_line( (int) $block['attrs']['id'] );
	if ( '' === $line ) {
		return $content;
	}
	$details = '<span class="picasso-places-camera">' . esc_html( $line ) . '</span>';
	$end     = strrpos( $content, '</figcaption>' );
	if ( false !== $end ) {
		return substr_replace( $content, $details, $end, 0 );
	}
	$end = strrpos( $content, '</figure>' );
	return false === $end ? $content : substr_replace( $content, '<figcaption class="wp-element-caption">' . $details . '</figcaption>', $end, 0 );
}
add_filter( 'render_block_core/image', __NAMESPACE__ . '\\image_camera', 10, 2 );

/**
 * Gives a Table block the column widths chosen in Picasso, which keeps them
 * in percent in its own attribute, as a <colgroup>.
 */
function table_widths( $content, $block ) {
	$widths = isset( $block['attrs']['picassoColumnWidths'] ) && is_array( $block['attrs']['picassoColumnWidths'] ) ? $block['attrs']['picassoColumnWidths'] : array();
	if ( ! $widths || false !== strpos( $content, '<colgroup' ) ) {
		return $content;
	}
	$columns = '';
	foreach ( $widths as $width ) {
		$columns .= '<col style="width:' . max( 0, min( 100, (int) $width ) ) . '%">';
	}
	return preg_replace( '~<table\b[^>]*>~', '$0<colgroup>' . $columns . '</colgroup>', $content, 1 );
}
add_filter( 'render_block_core/table', __NAMESPACE__ . '\\table_widths', 10, 2 );

/** Keep Picasso's column widths when the block editor saves a table. */
function table_width_attributes( $args, $block_type ) {
	if ( 'core/table' === $block_type ) {
		$args['attributes']['picassoColumnWidths'] = array( 'type' => 'array' );
	}
	return $args;
}
add_filter( 'register_block_type_args', __NAMESPACE__ . '\\table_width_attributes', 10, 2 );

/**
 * Starts an embedded YouTube video where its address says, such as t=90s
 * or t=1m30s: the player YouTube gives for an address leaves that out.
 */
function youtube_start( $html, $url ) {
	if ( ! is_string( $html ) || ! is_string( $url ) || ! preg_match( '~^https?://(?:[a-z]+\.)?(?:youtube\.com|youtu\.be)/~i', $url ) ) {
		return $html;
	}
	if ( ! preg_match( '~[?&#](?:amp;)?(?:t|start)=(?:(\d+)h)?(?:(\d+)m)?(?:(\d+)s?)?(?:&|#|$)~', $url, $time ) ) {
		return $html;
	}
	$start = 3600 * (int) ( $time[1] ?? 0 ) + 60 * (int) ( $time[2] ?? 0 ) + (int) ( $time[3] ?? 0 );
	if ( $start <= 0 ) {
		return $html;
	}
	return preg_replace_callback(
		'~(<iframe\b[^>]*?\bsrc=")(https://www\.youtube(?:-nocookie)?\.com/embed/[^"?]+)(\?[^"]*)?"~',
		function ( $found ) use ( $start ) {
			$query = $found[3] ?? '';
			if ( preg_match( '~[?&;]start=~', $query ) ) {
				return $found[0];
			}
			return $found[1] . $found[2] . ( '' === $query ? '?' : $query . '&amp;' ) . 'start=' . $start . '"';
		},
		$html,
		1
	);
}
add_filter( 'embed_oembed_html', __NAMESPACE__ . '\\youtube_start', 10, 2 );

/** The same camera line under the featured image of a single story or Journey. */
function thumbnail_camera( $html, $post_id, $thumb_id ) {
	if ( ! is_singular( array( 'post', 'vl_trip' ) ) || ! in_the_loop() || (int) get_the_ID() !== (int) $post_id ) {
		return $html;
	}
	$line = camera_line( (int) $thumb_id );
	if ( '' === $line ) {
		return $html;
	}
	return $html . '<span class="picasso-places-camera">' . esc_html( $line ) . '</span>';
}
add_filter( 'post_thumbnail_html', __NAMESPACE__ . '\\thumbnail_camera', 10, 3 );
