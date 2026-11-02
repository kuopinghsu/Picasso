<?php
/**
 * The geotag fields, the Journey post type and their REST API, the same as the
 * VoyageLight theme registers them, so Picasso can publish to any theme.
 *
 * @package PicassoPlaces
 */

namespace PicassoPlaces;

if ( ! defined( 'ABSPATH' ) ) {
	exit;
}

function parse_stops( $text ) {
	$stops = array();
	foreach ( vl_geo_lines( $text, STOP_LINES ) as $line ) {
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
function clean_lines( $text, $max_lines, $max_chars ) {
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

function meta_auth( $allowed, $meta_key, $object_id ) {
	return current_user_can( 'edit_post', $object_id );
}

function sanitize_lat( $value ) {
	return vl_valid_coord( $value, 'lat' );
}

function sanitize_lon( $value ) {
	return vl_valid_coord( $value, 'lon' );
}

function sanitize_stops( $value ) {
	return clean_lines( $value, STOP_LINES, STOP_CHARS );
}

function sanitize_trip_days( $value ) {
	return clean_lines( $value, DAY_LINES, DAY_CHARS );
}

function sanitize_trip_id( $value ) {
	$id = is_scalar( $value ) ? absint( $value ) : 0;
	return $id && 'vl_trip' === get_post_type( $id ) ? $id : null;
}

/** The geotag fields, also for publishing through the REST API. */
function register_meta() {
	$shared = array(
		'single'        => true,
		'show_in_rest'  => true,
		'auth_callback' => __NAMESPACE__ . '\\meta_auth',
	);
	register_post_meta( 'post', '_vl_lat', $shared + array( 'type' => 'number', 'sanitize_callback' => __NAMESPACE__ . '\\sanitize_lat' ) );
	register_post_meta( 'post', '_vl_lon', $shared + array( 'type' => 'number', 'sanitize_callback' => __NAMESPACE__ . '\\sanitize_lon' ) );
	register_post_meta( 'post', '_vl_stops', $shared + array( 'type' => 'string', 'sanitize_callback' => __NAMESPACE__ . '\\sanitize_stops' ) );
	register_post_meta( 'post', '_vl_trip_id', $shared + array( 'type' => 'integer', 'sanitize_callback' => __NAMESPACE__ . '\\sanitize_trip_id' ) );
	register_post_meta( 'vl_trip', '_vl_trip_days', $shared + array( 'type' => 'string', 'sanitize_callback' => __NAMESPACE__ . '\\sanitize_trip_days' ) );
}
add_action( 'init', __NAMESPACE__ . '\\register_meta' );

/** A geotag field that sanitizes to nothing, such as a latitude out of range, is removed rather than stored empty. */
function meta_drop_empty( $check, $object_id, $meta_key, $meta_value ) {
	if ( in_array( $meta_key, array( '_vl_lat', '_vl_lon', '_vl_stops', '_vl_trip_id', '_vl_trip_days' ), true ) && ( null === $meta_value || '' === $meta_value ) ) {
		delete_metadata( 'post', $object_id, $meta_key );
		return true;
	}
	return $check;
}
add_filter( 'add_post_metadata', __NAMESPACE__ . '\\meta_drop_empty', 10, 4 );
add_filter( 'update_post_metadata', __NAMESPACE__ . '\\meta_drop_empty', 10, 4 );

/**
 * REST shows a missing coordinate as 0. Sending 0, 0 back with no stop line
 * means no location. A stop line keeps a place at 0, 0.
 */
function rest_zero_location( $prepared, $request ) {
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
add_filter( 'rest_pre_insert_post', __NAMESPACE__ . '\\rest_zero_location', 10, 2 );

/** Journeys: a trip with day-by-day stops, travel stories and a map. */
function register_trips() {
	register_post_type(
		'vl_trip',
		array(
			'labels'       => array(
				'name'          => __( 'Journeys', 'picasso-places' ),
				'singular_name' => __( 'Journey', 'picasso-places' ),
				'add_new_item'  => __( 'Add Journey', 'picasso-places' ),
				'edit_item'     => __( 'Edit Journey', 'picasso-places' ),
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
add_action( 'init', __NAMESPACE__ . '\\register_trips' );

function trip_parse_days( $raw ) {
	$rows = array();
	foreach ( vl_geo_lines( $raw, DAY_LINES ) as $line ) {
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

function trip_stories( $id ) {
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

function day_label( $day ) {
	/* translators: %s: day number in a journey. */
	return sprintf( __( 'Day %s', 'picasso-places' ), number_format_i18n( $day ) );
}

/** The URL of a published story, or '' for anything else. */
function story_url( $id ) {
	return $id && 'post' === get_post_type( $id ) && 'publish' === get_post_status( $id ) ? get_permalink( $id ) : '';
}

function trip_points( $trip_id ) {
	$points = array();
	foreach ( trip_parse_days( get_post_meta( $trip_id, '_vl_trip_days', true ) ) as $row ) {
		$point = array(
			'name' => $row['name'],
			'lat'  => $row['lat'],
			'lon'  => $row['lon'],
			'note' => day_label( $row['day'] ) . ( $row['note'] ? ' · ' . $row['note'] : '' ),
		);
		$url   = story_url( $row['post_id'] );
		if ( $url ) {
			$point['url'] = $url;
		}
		$points[] = $point;
	}
	return $points;
}

function post_points( $id ) {
	$points = parse_stops( get_post_meta( $id, '_vl_stops', true ) );
	if ( $points ) {
		return $points;
	}
	$lat = vl_valid_coord( get_post_meta( $id, '_vl_lat', true ), 'lat' );
	$lon = vl_valid_coord( get_post_meta( $id, '_vl_lon', true ), 'lon' );
	return null !== $lat && null !== $lon ? array( array( 'name' => plain_title( $id ), 'lat' => $lat, 'lon' => $lon, 'note' => '' ) ) : array();
}

/** A title as plain text, for the scripts and for escaping once on output. */
function plain_title( $post ) {
	return html_entity_decode( wp_strip_all_tags( get_the_title( $post ) ), ENT_QUOTES, 'UTF-8' );
}
