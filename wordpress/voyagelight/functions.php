<?php
/**
 * VoyageLight theme setup, travel taxonomies, geotags, maps and Journeys.
 *
 * @package VoyageLight
 */

if ( ! defined( 'ABSPATH' ) ) {
	exit;
}

define( 'VL_VERSION', '1.6.0' );
define( 'VL_STOP_LINES', 500 );
define( 'VL_STOP_CHARS', 80000 );
define( 'VL_DAY_LINES', 5000 );
define( 'VL_DAY_CHARS', 400000 );

require_once get_template_directory() . '/inc/geo.php';

function vl_setup() {
	load_theme_textdomain( 'voyagelight', get_template_directory() . '/languages' );
	add_theme_support( 'title-tag' );
	add_theme_support( 'post-thumbnails' );
	add_theme_support( 'responsive-embeds' );
	add_theme_support( 'automatic-feed-links' );
	add_theme_support( 'custom-logo', array( 'height' => 80, 'width' => 220, 'flex-height' => true, 'flex-width' => true ) );
	add_theme_support( 'html5', array( 'search-form', 'comment-form', 'comment-list', 'gallery', 'caption', 'style', 'script' ) );
	add_theme_support( 'align-wide' );
	add_theme_support( 'wp-block-styles' );
	add_theme_support( 'editor-styles' );
	add_editor_style( 'assets/css/editor.css' );
	register_nav_menus(
		array(
			'primary' => __( 'Primary navigation', 'voyagelight' ),
			'footer'  => __( 'Footer navigation', 'voyagelight' ),
		)
	);
	add_image_size( 'vl-card', 900, 650, true );
	add_image_size( 'vl-hero', 1800, 900, true );
}
add_action( 'after_setup_theme', 'vl_setup' );

/** Text the scripts show, in the site's language. */
function vl_script_text() {
	return array(
		'dark'       => __( 'Dark', 'voyagelight' ),
		'light'      => __( 'Light', 'voyagelight' ),
		'loadingMap' => __( 'Loading map…', 'voyagelight' ),
		'retryMap'   => __( 'Retry loading the map', 'voyagelight' ),
		'mapFailed'  => __( 'Unable to load the map. Check your connection or content blockers.', 'voyagelight' ),
		'place'      => __( 'Place', 'voyagelight' ),
		'readStory'  => __( 'Read story ↗', 'voyagelight' ),
		'allStops'   => __( 'All stops', 'voyagelight' ),
		/* translators: %s: day number in a journey. */
		'day'        => __( 'Day %s', 'voyagelight' ),
	);
}

function vl_assets() {
	$uri = get_template_directory_uri();
	wp_enqueue_style( 'vl-theme', $uri . '/assets/css/theme.css', array(), VL_VERSION );
	wp_enqueue_script( 'vl-theme', $uri . '/assets/js/theme.js', array(), VL_VERSION, true );
	wp_localize_script( 'vl-theme', 'vlText', vl_script_text() );
	wp_enqueue_script( 'vl-map', $uri . '/assets/js/map.js', array( 'vl-theme' ), VL_VERSION, true );
	if ( is_singular( 'vl_trip' ) ) {
		wp_enqueue_script( 'vl-trip-days', $uri . '/assets/js/trip-days.js', array( 'vl-theme' ), VL_VERSION, true );
	}
	if ( is_singular() && comments_open() && get_option( 'thread_comments' ) ) {
		wp_enqueue_script( 'comment-reply' );
	}
}
add_action( 'wp_enqueue_scripts', 'vl_assets' );

/** Sets light or dark before the page is drawn; theme.js runs only after it. */
function vl_early_theme() {
	wp_print_inline_script_tag( "try{var t=localStorage.getItem('vl-theme');if(t!=='dark'&&t!=='light'){t=matchMedia('(prefers-color-scheme: dark)').matches?'dark':'light';}document.documentElement.setAttribute('data-theme',t);}catch(e){}" );
}
add_action( 'wp_head', 'vl_early_theme', 1 );

/** The start page's photo: the one chosen in the Customizer, at the hero size when it is in the media library. */
function vl_hero_url() {
	$url = get_theme_mod( 'vl_hero_image', '' );
	if ( '' === $url ) {
		return vl_photo( 'hero-norway' );
	}
	$id    = attachment_url_to_postid( $url );
	$sized = $id ? wp_get_attachment_image_url( $id, 'vl-hero' ) : false;
	return $sized ? $sized : $url;
}

function vl_preload_hero() {
	if ( is_front_page() ) {
		echo '<link rel="preload" as="image" href="' . esc_url( vl_hero_url() ) . '" fetchpriority="high">' . "\n";
	}
}
add_action( 'wp_head', 'vl_preload_hero', 2 );

function vl_taxonomies() {
	register_taxonomy(
		'destination',
		'post',
		array(
			'labels'            => array( 'name' => __( 'Destinations', 'voyagelight' ), 'singular_name' => __( 'Destination', 'voyagelight' ) ),
			'public'            => true,
			'show_in_rest'      => true,
			'hierarchical'      => true,
			'show_admin_column' => true,
			'rewrite'           => array( 'slug' => 'destination' ),
		)
	);
	register_taxonomy(
		'travel_type',
		'post',
		array(
			'labels'            => array( 'name' => __( 'Place types', 'voyagelight' ), 'singular_name' => __( 'Place type', 'voyagelight' ) ),
			'public'            => true,
			'show_in_rest'      => true,
			'hierarchical'      => true,
			'show_admin_column' => true,
			'rewrite'           => array( 'slug' => 'place-type' ),
		)
	);
}
add_action( 'init', 'vl_taxonomies' );

/**
 * Homepage headline, subtitle and image, and whether photos show their camera
 * details. Left empty, the headline and subtitle follow the site's language.
 */
function vl_customize( $customizer ) {
	$customizer->add_section( 'vl_home', array( 'title' => __( 'VoyageLight homepage', 'voyagelight' ), 'priority' => 35 ) );
	$fields = array(
		'hero_title'    => __( 'Hero headline', 'voyagelight' ),
		'hero_subtitle' => __( 'Hero subtitle', 'voyagelight' ),
	);
	foreach ( $fields as $key => $label ) {
		$customizer->add_setting( 'vl_' . $key, array( 'default' => '', 'sanitize_callback' => 'sanitize_text_field' ) );
		$customizer->add_control( 'vl_' . $key, array( 'section' => 'vl_home', 'label' => $label, 'type' => 'text' ) );
	}
	$customizer->add_setting( 'vl_hero_image', array( 'default' => '', 'sanitize_callback' => 'esc_url_raw' ) );
	$customizer->add_control( new WP_Customize_Image_Control( $customizer, 'vl_hero_image', array( 'section' => 'vl_home', 'label' => __( 'Hero image', 'voyagelight' ) ) ) );
	$customizer->add_setting( 'vl_show_camera', array( 'default' => true, 'sanitize_callback' => 'wp_validate_boolean' ) );
	$customizer->add_control( 'vl_show_camera', array( 'section' => 'vl_home', 'label' => __( 'Show camera details under photos', 'voyagelight' ), 'type' => 'checkbox' ) );
}
add_action( 'customize_register', 'vl_customize' );

/** A theme mod, or `$fallback` while it is empty. */
function vl_mod( $name, $fallback ) {
	$value = get_theme_mod( $name, '' );
	return '' === $value ? $fallback : $value;
}

/** URL of a bundled demo photo; unknown names give the hero photo. */
function vl_photo( $slug = 'hero-norway' ) {
	$allowed = array( 'hero-norway', 'stories', 'destinations', 'journeys', 'travel-map', 'food-culture', 'guides' );
	if ( ! in_array( $slug, $allowed, true ) ) {
		$slug = 'hero-norway';
	}
	return get_template_directory_uri() . '/assets/images/photos/' . $slug . '.jpg';
}

/** URL of a bundled illustration; unknown names give the Arctic one. */
function vl_image( $slug = 'arctic' ) {
	$allowed = array( 'arctic', 'baltic', 'greenland', 'japan', 'nordic', 'spain' );
	if ( ! in_array( $slug, $allowed, true ) ) {
		$slug = 'arctic';
	}
	return get_template_directory_uri() . '/assets/images/' . $slug . '.svg';
}

/** Minutes to read `$content`: words at 220 a minute, CJK characters at 400. */
function vl_read_minutes( $content ) {
	$text  = wp_strip_all_tags( $content );
	$cjk   = preg_match_all( '/[\x{3040}-\x{30FF}\x{3400}-\x{9FFF}\x{AC00}-\x{D7AF}]/u', $text );
	$words = str_word_count( preg_replace( '/[^\x00-\x7F]+/', ' ', $text ) );
	return max( 1, (int) ceil( $words / 220 + $cjk / 400 ) );
}

function vl_card( $post_id ) {
	$destinations = get_the_terms( $post_id, 'destination' );
	$destination  = ( $destinations && ! is_wp_error( $destinations ) ) ? $destinations[0]->name : __( 'Travel journal', 'voyagelight' );
	$categories   = get_the_category( $post_id );
	$category     = $categories ? $categories[0]->name : __( 'Story', 'voyagelight' );
	$link         = get_permalink( $post_id );
	// The title is the card's one link; CSS stretches it over the whole card.
	echo '<article class="story-card"><div class="story-visual">';
	if ( has_post_thumbnail( $post_id ) ) {
		echo get_the_post_thumbnail( $post_id, 'vl-card', array( 'loading' => 'lazy', 'alt' => '' ) );
	} else {
		echo '<img src="' . esc_url( vl_image( 'arctic' ) ) . '" alt="" loading="lazy">';
	}
	echo '<span class="story-badge">' . esc_html( $destination ) . '</span></div>';
	echo '<div class="story-copy"><span class="eyebrow">' . esc_html( $category ) . ' · ' . esc_html( get_the_date( '', $post_id ) ) . '</span>';
	echo '<h3><a href="' . esc_url( $link ) . '">' . esc_html( get_the_title( $post_id ) ) . '</a></h3>';
	echo '<p>' . esc_html( wp_trim_words( get_the_excerpt( $post_id ), 20 ) ) . '</p></div></article>';
}

function vl_body_class( $classes ) {
	$classes[] = 'vl-site';
	return $classes;
}
add_filter( 'body_class', 'vl_body_class' );

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
	echo '<span class="description">' . esc_html__( 'The map loads Leaflet from unpkg.com and map tiles from OpenStreetMap, which see your IP address.', 'voyagelight' ) . '</span></p>';
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
	$value   = null !== $lat && null !== $lon ? $lat . ', ' . $lon : '';
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

function vl_post_points( $id ) {
	$points = vl_parse_stops( get_post_meta( $id, '_vl_stops', true ) );
	if ( $points ) {
		return $points;
	}
	$lat = vl_valid_coord( get_post_meta( $id, '_vl_lat', true ), 'lat' );
	$lon = vl_valid_coord( get_post_meta( $id, '_vl_lon', true ), 'lon' );
	return null !== $lat && null !== $lon ? array( array( 'name' => get_the_title( $id ), 'lat' => $lat, 'lon' => $lon, 'note' => '' ) ) : array();
}

function vl_map_data( $scope = 'all', $post_id = 0 ) {
	if ( 'post' === $scope ) {
		return vl_post_points( $post_id );
	}
	// Only published posts are shown on the atlas.
	// Whole posts, not IDs, so their fields and meta come in one query each.
	$points = array();
	foreach (
		vl_collect_posts(
			array(
				'post_type'   => 'post',
				'post_status' => 'publish',
				'meta_query'  => array( array( 'key' => '_vl_lat', 'compare' => 'EXISTS' ) ),
			)
		) as $post
	) {
		$lat = vl_valid_coord( get_post_meta( $post->ID, '_vl_lat', true ), 'lat' );
		$lon = vl_valid_coord( get_post_meta( $post->ID, '_vl_lon', true ), 'lon' );
		if ( null !== $lat && null !== $lon ) {
			$points[] = array( 'name' => get_the_title( $post ), 'lat' => $lat, 'lon' => $lon, 'note' => '', 'url' => get_permalink( $post ) );
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

function vl_render_map( $points, $heading, $route = false ) {
	if ( ! $points ) {
		return '';
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
	return '<section class="vl-map-section"><div class="vl-map-heading"><div><span class="eyebrow">' . esc_html__( 'GEOGRAPHIC CONTEXT', 'voyagelight' ) . '</span>'
		. '<h2>' . esc_html( $heading ) . '</h2>'
		. '<p>' . esc_html__( 'See where the places are in relation to each other, and select a pin to read its note. Lines show the order of stops, not the actual route.', 'voyagelight' ) . '</p></div></div>'
		. '<div class="vl-map" data-map="' . esc_attr( $json ) . '"><div class="vl-map-placeholder">'
		/* translators: %d: number of places on the map. */
		. '<p>' . esc_html( sprintf( _n( 'Interactive map · %d place', 'Interactive map · %d places', $count, 'voyagelight' ), $count ) ) . '</p>'
		. '<button class="vl-map-load" type="button">' . esc_html__( 'Load map', 'voyagelight' ) . '</button>'
		. '<small>' . esc_html__( 'Uses OpenStreetMap tiles and Leaflet from external services. Loading shares your IP address with them.', 'voyagelight' ) . '</small></div></div>'
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
			'rewrite'      => array( 'slug' => 'trips' ),
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
			'location' => $lat . ', ' . $lon,
		);
	}
	$others = vl_collect_posts(
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
			$stories[] = array( 'id' => $story->ID, 'title' => vl_plain_title( $story ), 'linked' => false );
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
		echo '<img src="' . esc_url( vl_image( 'nordic' ) ) . '" alt="" loading="lazy">';
	}
	echo '<span class="story-badge">' . esc_html__( 'Journey', 'voyagelight' ) . '</span></div>';
	/* translators: %d: number of stops in a journey. */
	echo '<div class="story-copy"><span class="eyebrow">' . esc_html( sprintf( _n( '%d stop', '%d stops', $stops, 'voyagelight' ), $stops ) ) . '</span>';
	echo '<h3><a href="' . esc_url( get_permalink( $id ) ) . '">' . esc_html( get_the_title( $id ) ) . '</a></h3>';
	echo '<p>' . esc_html( wp_trim_words( get_the_excerpt( $id ), 24 ) ) . '</p></div></article>';
}

/**
 * How an uploaded photo was taken, such as "Canon EOS R6 · 35 mm · f/2.8 ·
 * 1/250 s · ISO 400", from what WordPress read of its EXIF; '' when it has none
 * or the Customizer turns them off. Picasso sends them only with Keep camera details.
 */
function vl_camera_line( $attachment_id ) {
	if ( ! get_theme_mod( 'vl_show_camera', true ) ) {
		return '';
	}
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
function vl_mark_gallery_images( $block, $source, $parent = null ) {
	if ( 'core/image' === $block['blockName'] && $parent instanceof WP_Block && 'core/gallery' === $parent->name ) {
		$block['attrs']['vlInGallery'] = true;
	}
	return $block;
}
add_filter( 'render_block_data', 'vl_mark_gallery_images', 10, 3 );

/** Adds the camera details to an Image block's caption, making one if it has none. */
function vl_image_camera( $content, $block ) {
	if ( empty( $block['attrs']['id'] ) || ! empty( $block['attrs']['vlInGallery'] ) ) {
		return $content;
	}
	$line = vl_camera_line( (int) $block['attrs']['id'] );
	if ( '' === $line ) {
		return $content;
	}
	$details = '<span class="vl-camera">' . esc_html( $line ) . '</span>';
	$end     = strrpos( $content, '</figcaption>' );
	if ( false !== $end ) {
		return substr_replace( $content, $details, $end, 0 );
	}
	$end = strrpos( $content, '</figure>' );
	return false === $end ? $content : substr_replace( $content, '<figcaption class="wp-element-caption">' . $details . '</figcaption>', $end, 0 );
}
add_filter( 'render_block_core/image', 'vl_image_camera', 10, 2 );
