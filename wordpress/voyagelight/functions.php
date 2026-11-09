<?php
/**
 * VoyageLight theme setup: language, assets, light and dark, the start page's photo and the travel taxonomies.
 * The rest is in inc/: customizer.php, text-changes.php, content.php, locations.php, journeys.php and media.php.
 *
 * @package VoyageLight
 */

if ( ! defined( 'ABSPATH' ) ) {
	exit;
}

define( 'VL_VERSION', '1.7.0' );
define( 'VL_STOP_LINES', 500 );
define( 'VL_STOP_CHARS', 80000 );
define( 'VL_DAY_LINES', 5000 );
define( 'VL_DAY_CHARS', 400000 );

require_once get_template_directory() . '/inc/geo.php';
require_once get_template_directory() . '/inc/discovery.php';
require_once get_template_directory() . '/inc/customizer.php';
require_once get_template_directory() . '/inc/text-changes.php';
require_once get_template_directory() . '/inc/content.php';
require_once get_template_directory() . '/inc/locations.php';
require_once get_template_directory() . '/inc/journeys.php';
require_once get_template_directory() . '/inc/media.php';

/**
 * Loads the theme's language. `make theme` makes an English and a Traditional Chinese build, each
 * with inc/language.php naming its language, which the theme then shows whatever the site's language.
 * Without that file, as in the source, the theme follows Settings > General > Site Language.
 */
function vl_load_language() {
	$file     = get_template_directory() . '/inc/language.php';
	$language = is_readable( $file ) ? (string) include $file : '';
	if ( '' === $language ) {
		load_theme_textdomain( 'voyagelight', get_template_directory() . '/languages' );
		return;
	}
	$mo = get_template_directory() . '/languages/' . $language . '.mo';
	if ( is_readable( $mo ) ) {
		load_textdomain( 'voyagelight', $mo );
	}
}

function vl_setup() {
	vl_load_language();
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
	$accent = vl_accent_css();
	if ( '' !== $accent ) {
		wp_add_inline_style( 'vl-theme', $accent );
	}
	wp_enqueue_script( 'vl-theme', $uri . '/assets/js/theme.js', array(), VL_VERSION, true );
	wp_localize_script(
		'vl-theme',
		'vlText',
		array_merge(
			vl_script_text(),
			array(
				'leaflet'     => $uri . '/assets/vendor/leaflet/',
				'colorMode'   => vl_color_mode(),
				'colorToggle' => vl_color_toggle() ? '1' : '',
			)
		)
	);
	wp_enqueue_script( 'vl-map', $uri . '/assets/js/map.js', array( 'vl-theme' ), VL_VERSION, true );
	if ( is_singular( 'vl_trip' ) ) {
		wp_enqueue_script( 'vl-trip-days', $uri . '/assets/js/trip-days.js', array( 'vl-theme' ), VL_VERSION, true );
	}
	if ( is_singular() && comments_open() && get_option( 'thread_comments' ) ) {
		wp_enqueue_script( 'comment-reply' );
	}
}
add_action( 'wp_enqueue_scripts', 'vl_assets' );

/** The colours a visitor first sees, from Appearance > Customize > Colours: system, light or dark. */
function vl_color_mode() {
	return vl_sanitize_color_mode( get_theme_mod( 'vl_color_mode', 'system' ) );
}

function vl_sanitize_color_mode( $value ) {
	return in_array( $value, array( 'system', 'light', 'dark' ), true ) ? $value : 'system';
}

/** Whether the header has the button that switches between light and dark. */
function vl_color_toggle() {
	return (bool) get_theme_mod( 'vl_color_toggle', true );
}

/** Where story lists and the start page's latest stories have their sidebar: right, left or none. */
function vl_sidebar_position() {
	return vl_sanitize_sidebar( get_theme_mod( 'vl_sidebar', 'right' ) );
}

function vl_sanitize_sidebar( $value ) {
	return in_array( $value, array( 'right', 'left', 'none' ), true ) ? $value : 'right';
}

/** The sidebar's place as a class on the page, for the stylesheet. */
function vl_sidebar_class( $classes ) {
	$classes[] = 'vl-sidebar-' . vl_sidebar_position();
	return $classes;
}
add_filter( 'body_class', 'vl_sidebar_class' );

/**
 * A fixed light or dark, with no button to change it, is written into the page itself,
 * so it holds even before the script runs or without it.
 */
function vl_fixed_theme( $output ) {
	return ! vl_color_toggle() && 'system' !== vl_color_mode() ? $output . ' data-theme="' . esc_attr( vl_color_mode() ) . '"' : $output;
}
add_filter( 'language_attributes', 'vl_fixed_theme' );

/**
 * Sets light or dark before the page is drawn; theme.js runs only after it. A visitor's own
 * choice comes first while the header has the button, then the Customizer's colour mode.
 */
function vl_early_theme() {
	$mode   = wp_json_encode( vl_color_mode() );
	$toggle = vl_color_toggle() ? 'true' : 'false';
	wp_print_inline_script_tag( "(function(){var d={$mode},t=null;try{if({$toggle}){t=localStorage.getItem('vl-theme');}}catch(e){}if(t!=='dark'&&t!=='light'){t=d==='system'?(window.matchMedia&&matchMedia('(prefers-color-scheme: dark)').matches?'dark':'light'):d;}document.documentElement.setAttribute('data-theme',t);}());" );
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

/**
 * The start page's photo as an image behind the headline, fetched first. A photo from the media
 * library comes with its smaller sizes, so a phone downloads one its own size.
 */
function vl_hero_image() {
	$attrs = array(
		'class'         => 'hero-image',
		'alt'           => '',
		'sizes'         => '100vw',
		'loading'       => 'eager',
		'fetchpriority' => 'high',
		'decoding'      => 'async',
	);
	$url = get_theme_mod( 'vl_hero_image', '' );
	$id  = '' !== $url ? attachment_url_to_postid( $url ) : 0;
	if ( $id ) {
		$image = wp_get_attachment_image( $id, 'vl-hero', false, $attrs );
		if ( '' !== $image ) {
			return $image;
		}
	}
	return '<img class="hero-image" src="' . esc_url( '' !== $url ? $url : vl_photo( 'hero-norway' ) ) . '" alt="" fetchpriority="high" decoding="async">';
}

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
			'rewrite'           => array( 'slug' => vl_permalink_base( 'destination' ) ),
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
			'rewrite'           => array( 'slug' => vl_permalink_base( 'place_type' ) ),
		)
	);
}
add_action( 'init', 'vl_taxonomies' );
