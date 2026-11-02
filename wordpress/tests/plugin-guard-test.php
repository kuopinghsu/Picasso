<?php
/**
 * Checks that the Picasso Places plugin keeps out of the VoyageLight theme's
 * way: the plugin and then the theme's functions.php load in one process, as
 * WordPress loads them, without a "cannot redeclare" fatal; the plugin hooks
 * nothing while VoyageLight (or a child of it) is the theme and everything
 * otherwise; and it declares functions only in its own namespace, and no
 * classes or global constants. WordPress isn't needed: the few functions the
 * files call while loading are stubbed below.
 *
 *     php wordpress/tests/plugin-guard-test.php
 */

define( 'ABSPATH', __DIR__ . '/' );

$hooks    = array();
$calls    = array();
$template = 'voyagelight';
$failed   = 0;
$passed   = 0;

function add_action( $hook, $callback ) {
	$GLOBALS['hooks'][] = array( $hook, is_string( $callback ) ? $callback : 'closure' );
	return true;
}
function add_filter( $hook, $callback ) {
	return add_action( $hook, $callback );
}
function add_shortcode( $tag, $callback ) {
	return add_action( 'shortcode ' . $tag, $callback );
}
function register_activation_hook( $file, $callback ) {
	$GLOBALS['activate'] = $callback;
}
function register_deactivation_hook( $file, $callback ) {
	$GLOBALS['deactivate'] = $callback;
}
function get_template() {
	return $GLOBALS['template'];
}
function get_template_directory() {
	return __DIR__ . '/../voyagelight';
}
function plugin_basename( $file ) {
	return 'picasso-places/' . basename( $file );
}
function load_plugin_textdomain( $domain ) {
	$GLOBALS['calls'][] = "load_plugin_textdomain $domain";
	return true;
}
function delete_option( $name ) {
	$GLOBALS['calls'][] = "delete_option $name";
	return true;
}

function guard_check( $name, $ok ) {
	global $failed, $passed;
	if ( $ok ) {
		++$passed;
		return;
	}
	++$failed;
	fwrite( STDERR, "FAIL $name\n" );
}

/** The plugin's hooks, as "hook callback" lines. */
function plugin_hooks() {
	$out = array();
	foreach ( $GLOBALS['hooks'] as $hook ) {
		if ( 0 === strpos( $hook[1], 'PicassoPlaces\\' ) ) {
			$out[] = $hook[0] . ' ' . $hook[1];
		}
	}
	return $out;
}

/** What WordPress does at after_setup_theme, for the plugin's callbacks only; the theme's need WordPress. */
function after_setup_theme() {
	foreach ( $GLOBALS['hooks'] as $hook ) {
		if ( 'after_setup_theme' === $hook[0] && 0 === strpos( $hook[1], 'PicassoPlaces\\' ) ) {
			call_user_func( $hook[1] );
		}
	}
}

$plugin = realpath( __DIR__ . '/../picasso-places' );
require $plugin . '/picasso-places.php';
require get_template_directory() . '/functions.php';
guard_check( 'the theme loads after the plugin', function_exists( 'vl_register_meta' ) && function_exists( 'vl_parse_location' ) );
guard_check( 'the plugin waits for after_setup_theme', array( 'after_setup_theme PicassoPlaces\\start' ) === plugin_hooks() );

$template = 'voyagelight';
after_setup_theme();
call_user_func( $activate );
call_user_func( $deactivate );
guard_check( 'with VoyageLight the plugin adds no hooks', array( 'after_setup_theme PicassoPlaces\\start' ) === plugin_hooks() );
guard_check( 'with VoyageLight the plugin loads nothing more', array( $plugin . '/picasso-places.php' ) === array_values( preg_grep( '~^' . preg_quote( $plugin, '~' ) . '/~', get_included_files() ) ) );
guard_check( 'with VoyageLight activating and deactivating change nothing', array() === $calls );

$template = 'twentytwentyfive';
after_setup_theme();
$added = plugin_hooks();
foreach ( array( 'init PicassoPlaces\\register_meta', 'init PicassoPlaces\\register_trips', 'rest_pre_insert_post PicassoPlaces\\rest_zero_location', 'add_meta_boxes PicassoPlaces\\editor_boxes', 'admin_enqueue_scripts PicassoPlaces\\admin_assets', 'wp_enqueue_scripts PicassoPlaces\\front_assets', 'the_content PicassoPlaces\\add_to_content', 'render_block_data PicassoPlaces\\mark_gallery_images', 'render_block_core/image PicassoPlaces\\image_camera', 'post_thumbnail_html PicassoPlaces\\thumbnail_camera' ) as $hook ) {
	guard_check( "with another theme the plugin hooks $hook", in_array( $hook, $added, true ) );
}
guard_check( 'with another theme the plugin loads its translations', in_array( 'load_plugin_textdomain picasso-places', $calls, true ) );
call_user_func( $activate );
guard_check( 'with another theme activating resets the address rules', in_array( 'delete_option rewrite_rules', $calls, true ) );

foreach ( get_defined_functions()['user'] as $name ) {
	$file = ( new ReflectionFunction( $name ) )->getFileName();
	if ( 0 === strpos( $file, $plugin . '/' ) ) {
		guard_check( "$name is in the plugin's namespace", 0 === strpos( $name, 'picassoplaces\\' ) );
	}
}
foreach ( get_declared_classes() as $name ) {
	$file = ( new ReflectionClass( $name ) )->getFileName();
	guard_check( "the plugin declares no class ($name)", ! $file || 0 !== strpos( $file, $plugin . '/' ) );
}
foreach ( array_keys( get_defined_constants( true )['user'] ) as $name ) {
	guard_check( "constant $name is the theme's or in the plugin's namespace", 'ABSPATH' === $name || 0 === strpos( $name, 'VL_' ) || 0 === strpos( $name, 'PicassoPlaces\\' ) );
}
guard_check( "the plugin's reader and the theme's both work", array( 39.8628, -4.0273 ) === vl_parse_location( '39.8628, -4.0273' ) && array( 39.8628, -4.0273 ) === PicassoPlaces\vl_parse_location( '39.8628, -4.0273' ) );

echo "$passed passed, $failed failed\n";
exit( $failed ? 1 : 0 );
