<?php
/**
 * Plugin Name:       Picasso Places
 * Description:       Locations, route stops, maps and Journeys for travel stories, with any theme, including the fields Picasso publishes. It does nothing while the VoyageLight theme is active, which has them built in.
 * Version:           1.0.0
 * Requires at least: 6.2
 * Requires PHP:      7.4
 * Author:            Picasso
 * License:           GPLv2 or later
 * License URI:       https://www.gnu.org/licenses/gpl-2.0.html
 * Text Domain:       picasso-places
 * Domain Path:       /languages
 *
 * @package PicassoPlaces
 */

namespace PicassoPlaces;

if ( ! defined( 'ABSPATH' ) ) {
	exit;
}

const VERSION    = '1.0.0';
const STOP_LINES = 500;
const STOP_CHARS = 80000;
const DAY_LINES  = 5000;
const DAY_CHARS  = 400000;

/**
 * Whether the active theme is VoyageLight or a child of it, which registers
 * the same fields, post type, editor boxes and maps itself.
 */
function theme_provides_places() {
	return 'voyagelight' === get_template();
}

/**
 * Loads the rest of the plugin once the theme is known. VoyageLight's
 * functions.php has run by now, and the Customizer's theme preview is set.
 */
function start() {
	if ( theme_provides_places() ) {
		return;
	}
	load_plugin_textdomain( 'picasso-places', false, dirname( plugin_basename( __FILE__ ) ) . '/languages' );
	require_once __DIR__ . '/inc/geo.php';
	require_once __DIR__ . '/inc/places.php';
	require_once __DIR__ . '/inc/editor.php';
	require_once __DIR__ . '/inc/front.php';
}
add_action( 'after_setup_theme', __NAMESPACE__ . '\\start' );

/** The URL of a file in assets/. */
function asset_url( $path ) {
	return plugins_url( 'assets/' . $path, __FILE__ );
}

/** Rebuilds the address rules on the next request, so /trips/ comes and goes with the plugin. */
function reset_rewrites() {
	if ( ! theme_provides_places() ) {
		delete_option( 'rewrite_rules' );
	}
}
register_activation_hook( __FILE__, __NAMESPACE__ . '\\reset_rewrites' );
register_deactivation_hook( __FILE__, __NAMESPACE__ . '\\reset_rewrites' );
