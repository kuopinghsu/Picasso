<?php
/**
 * The Customizer's settings, the accent colour's shades and the start page's sections.
 *
 * @package VoyageLight
 */

if ( ! defined( 'ABSPATH' ) ) {
	exit;
}

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
	$customizer->add_setting( 'vl_editorial_image', array( 'default' => '', 'sanitize_callback' => 'esc_url_raw' ) );
	$customizer->add_control( new WP_Customize_Image_Control( $customizer, 'vl_editorial_image', array( 'section' => 'vl_home', 'label' => __( 'Collections photo', 'voyagelight' ) ) ) );
	$customizer->add_setting( 'vl_show_camera', array( 'default' => true, 'sanitize_callback' => 'wp_validate_boolean' ) );
	$customizer->add_control( 'vl_show_camera', array( 'section' => 'vl_home', 'label' => __( 'Show camera details under photos', 'voyagelight' ), 'type' => 'checkbox' ) );
	// The start page's sections, each shown unless turned off, and how many stories and Journeys it lists.
	foreach ( vl_home_sections() as $key => $label ) {
		$customizer->add_setting( 'vl_show_' . $key, array( 'default' => true, 'sanitize_callback' => 'wp_validate_boolean' ) );
		$customizer->add_control( 'vl_show_' . $key, array( 'section' => 'vl_home', 'label' => $label, 'type' => 'checkbox' ) );
	}
	require_once get_template_directory() . '/inc/customize-order.php';
	$customizer->add_setting( 'vl_home_order', array( 'default' => implode( ',', array_keys( vl_home_sections() ) ), 'sanitize_callback' => 'vl_sanitize_home_order' ) );
	$customizer->add_control(
		new VL_Order_Control(
			$customizer,
			'vl_home_order',
			array(
				'section'     => 'vl_home',
				'label'       => __( 'Order of sections', 'voyagelight' ),
				'description' => __( 'The large photo always comes first.', 'voyagelight' ),
			)
		)
	);
	$counts = array(
		'stories'  => array( 'vl_home', __( 'Stories on the start page', 'voyagelight' ), 5 ),
		'journeys' => array( 'vl_home', __( 'Journeys on the start page', 'voyagelight' ), 3 ),
		'related'  => array( 'vl_pages', __( 'Related stories after each story', 'voyagelight' ), 3 ),
		'footer'   => array( 'vl_pages', __( 'Journeys in the footer', 'voyagelight' ), 5 ),
	);
	foreach ( $counts as $key => $count ) {
		$customizer->add_setting( 'vl_' . $key . '_count', array( 'default' => $count[2], 'sanitize_callback' => 'vl_sanitize_count' ) );
		$customizer->add_control( 'vl_' . $key . '_count', array( 'section' => $count[0], 'label' => $count[1], 'type' => 'number', 'input_attrs' => array( 'min' => 1, 'max' => 12 ) ) );
	}
	$customizer->add_setting( 'vl_summary_words', array( 'default' => 20, 'sanitize_callback' => 'vl_sanitize_summary_words' ) );
	$customizer->add_control(
		'vl_summary_words',
		array(
			'section'     => 'vl_pages',
			'label'       => __( 'Length of card summaries', 'voyagelight' ),
			'description' => __( 'In words, from 5 to 80. In Chinese and Japanese, WordPress counts characters instead.', 'voyagelight' ),
			'type'        => 'number',
			'input_attrs' => array(
				'min' => 5,
				'max' => 80,
			),
		)
	);
	$customizer->add_setting( 'vl_accent', array( 'default' => '', 'sanitize_callback' => 'sanitize_hex_color' ) );
	$customizer->add_control(
		new WP_Customize_Color_Control(
			$customizer,
			'vl_accent',
			array(
				'section'     => 'colors',
				'label'       => __( 'Accent colour', 'voyagelight' ),
				'description' => __( 'For links, buttons and highlights. The theme makes its lighter and darker shades, and one for dark mode, dark or light enough to read. Leave it empty for the theme’s teal.', 'voyagelight' ),
			)
		)
	);
	$customizer->add_setting( 'vl_color_mode', array( 'default' => 'system', 'sanitize_callback' => 'vl_sanitize_color_mode' ) );
	$customizer->add_control(
		'vl_color_mode',
		array(
			'section' => 'colors',
			'label'   => __( 'Light or dark', 'voyagelight' ),
			'type'    => 'radio',
			'choices' => array(
				'system' => __( 'As the visitor’s device is set', 'voyagelight' ),
				'light'  => __( 'Light', 'voyagelight' ),
				'dark'   => __( 'Dark', 'voyagelight' ),
			),
		)
	);
	$customizer->add_setting( 'vl_color_toggle', array( 'default' => true, 'sanitize_callback' => 'wp_validate_boolean' ) );
	$customizer->add_control(
		'vl_color_toggle',
		array(
			'section'     => 'colors',
			'label'       => __( 'Let visitors switch between light and dark', 'voyagelight' ),
			'description' => __( 'A button in the header. A visitor’s choice is kept in their browser and wins over the setting above.', 'voyagelight' ),
			'type'        => 'checkbox',
		)
	);
	$customizer->add_section( 'vl_pages', array( 'title' => __( 'VoyageLight stories', 'voyagelight' ), 'priority' => 36 ) );
	$customizer->add_setting( 'vl_sidebar', array( 'default' => 'right', 'sanitize_callback' => 'vl_sanitize_sidebar' ) );
	$customizer->add_control(
		'vl_sidebar',
		array(
			'section'     => 'vl_pages',
			'label'       => __( 'Sidebar', 'voyagelight' ),
			'description' => __( 'Beside story lists and the start page’s latest stories.', 'voyagelight' ),
			'type'        => 'radio',
			'choices'     => array(
				'right' => __( 'On the right', 'voyagelight' ),
				'left'  => __( 'On the left', 'voyagelight' ),
				'none'  => __( 'None', 'voyagelight' ),
			),
		)
	);
	foreach ( array( 'breadcrumbs' => __( 'Show breadcrumbs', 'voyagelight' ), 'related' => __( 'Show related stories', 'voyagelight' ) ) as $key => $label ) {
		$customizer->add_setting( 'vl_show_' . $key, array( 'default' => true, 'sanitize_callback' => 'wp_validate_boolean' ) );
		$customizer->add_control( 'vl_show_' . $key, array( 'section' => 'vl_pages', 'label' => $label, 'type' => 'checkbox' ) );
	}
	$customizer->add_section( 'vl_text', array( 'title' => __( 'VoyageLight text', 'voyagelight' ), 'priority' => 36 ) );
	$customizer->add_setting( 'vl_text_changes', array( 'default' => '', 'sanitize_callback' => 'sanitize_textarea_field' ) );
	$customizer->add_control(
		'vl_text_changes',
		array(
			'section'     => 'vl_text',
			'label'       => __( 'Text changes', 'voyagelight' ),
			'description' => __( 'One change per line: the text as the site shows it, an equals sign, then the new text, such as “Latest travel stories = Recent posts”. Nothing after the equals sign hides the text. A text with %s or %d keeps them in the new text. Leave this empty for the theme’s own words.', 'voyagelight' ),
			'type'        => 'textarea',
			'input_attrs' => array( 'rows' => 12 ),
		)
	);
}
add_action( 'customize_register', 'vl_customize' );

/* ---------- Accent colour ---------- */

/** array( r, g, b ) of a #rgb or #rrggbb colour. */
function vl_hex_rgb( $hex ) {
	$hex = ltrim( $hex, '#' );
	if ( 3 === strlen( $hex ) ) {
		$hex = $hex[0] . $hex[0] . $hex[1] . $hex[1] . $hex[2] . $hex[2];
	}
	return array( hexdec( substr( $hex, 0, 2 ) ), hexdec( substr( $hex, 2, 2 ) ), hexdec( substr( $hex, 4, 2 ) ) );
}

/** #rrggbb of array( r, g, b ). */
function vl_rgb_hex( $rgb ) {
	return vsprintf( '#%02x%02x%02x', $rgb );
}

/** A colour's relative luminance, as WCAG measures it. */
function vl_luminance( $rgb ) {
	$sum = 0;
	foreach ( array( 0.2126, 0.7152, 0.0722 ) as $i => $weight ) {
		$value = $rgb[ $i ] / 255;
		$sum  += $weight * ( $value <= 0.03928 ? $value / 12.92 : pow( ( $value + 0.055 ) / 1.055, 2.4 ) );
	}
	return $sum;
}

/** The WCAG contrast ratio of two colours, from 1 to 21. */
function vl_contrast( $a, $b ) {
	$x = vl_luminance( $a );
	$y = vl_luminance( $b );
	return ( max( $x, $y ) + 0.05 ) / ( min( $x, $y ) + 0.05 );
}

/** array( hue, saturation, lightness ), each from 0 to 1, of array( r, g, b ). */
function vl_rgb_hsl( $rgb ) {
	list( $r, $g, $b ) = array( $rgb[0] / 255, $rgb[1] / 255, $rgb[2] / 255 );
	$max = max( $r, $g, $b );
	$min = min( $r, $g, $b );
	$l   = ( $max + $min ) / 2;
	if ( $max === $min ) {
		return array( 0.0, 0.0, $l );
	}
	$d = $max - $min;
	$s = $l > 0.5 ? $d / ( 2 - $max - $min ) : $d / ( $max + $min );
	if ( $max === $r ) {
		$h = ( $g - $b ) / $d + ( $g < $b ? 6 : 0 );
	} elseif ( $max === $g ) {
		$h = ( $b - $r ) / $d + 2;
	} else {
		$h = ( $r - $g ) / $d + 4;
	}
	return array( $h / 6, $s, $l );
}

/** One channel of vl_hsl_rgb(). */
function vl_hue_channel( $p, $q, $t ) {
	$t = $t < 0 ? $t + 1 : ( $t > 1 ? $t - 1 : $t );
	if ( $t < 1 / 6 ) {
		return $p + ( $q - $p ) * 6 * $t;
	}
	if ( $t < 1 / 2 ) {
		return $q;
	}
	if ( $t < 2 / 3 ) {
		return $p + ( $q - $p ) * ( 2 / 3 - $t ) * 6;
	}
	return $p;
}

/** array( r, g, b ) of a hue, saturation and lightness from 0 to 1. */
function vl_hsl_rgb( $h, $s, $l ) {
	if ( 0.0 === (float) $s ) {
		$v = (int) round( $l * 255 );
		return array( $v, $v, $v );
	}
	$q = $l < 0.5 ? $l * ( 1 + $s ) : $l + $s - $l * $s;
	$p = 2 * $l - $q;
	return array(
		(int) round( vl_hue_channel( $p, $q, $h + 1 / 3 ) * 255 ),
		(int) round( vl_hue_channel( $p, $q, $h ) * 255 ),
		(int) round( vl_hue_channel( $p, $q, $h - 1 / 3 ) * 255 ),
	);
}

/** `$rgb` made darker (or lighter) a step at a time until its contrast with `$against` reaches `$target`. */
function vl_readable_shade( $rgb, $against, $target, $darker ) {
	list( $h, $s, $l ) = vl_rgb_hsl( $rgb );
	for ( $i = 0; $i <= 100; $i++ ) {
		$shade = vl_hsl_rgb( $h, $s, $l );
		if ( vl_contrast( $shade, $against ) >= $target ) {
			return $shade;
		}
		$l = $darker ? max( 0, $l - 0.01 ) : min( 1, $l + 0.01 );
	}
	return vl_hsl_rgb( $h, $s, $l );
}

/**
 * The CSS for the accent colour chosen in Appearance > Customize > Colours, or '' for the theme's teal.
 * Light mode takes a shade dark enough for text on the page (5.2:1), dark mode one light enough (8:1);
 * each gets a hover shade, a tint, button text that reads on it and a table header in the same hue,
 * so every pick keeps text at WCAG AA contrast.
 */
function vl_accent_css() {
	$pick = sanitize_hex_color( get_theme_mod( 'vl_accent', '' ) );
	if ( ! $pick ) {
		return '';
	}
	$base           = vl_hex_rgb( $pick );
	list( $h, $s )  = vl_rgb_hsl( $base );
	$light          = vl_readable_shade( $base, vl_hex_rgb( '#f6f5f2' ), 5.2, true );
	$dark           = vl_readable_shade( $base, vl_hex_rgb( '#0b141b' ), 8, false );
	$light_hsl      = vl_rgb_hsl( $light );
	$dark_hsl       = vl_rgb_hsl( $dark );
	$on_light       = vl_contrast( array( 255, 255, 255 ), $light ) >= 4.5 ? '#fff' : '#10242e';
	$on_dark        = vl_contrast( vl_hex_rgb( '#06212b' ), $dark ) >= 4.5 ? '#06212b' : '#fff';
	$tint           = function ( $rgb, $alpha ) {
		return 'rgba(' . implode( ',', $rgb ) . ',' . $alpha . ')';
	};
	$css  = ':root{--accent:' . vl_rgb_hex( $light ) . ';--accent2:' . vl_rgb_hex( vl_hsl_rgb( $light_hsl[0], $light_hsl[1], max( 0, $light_hsl[2] - 0.08 ) ) ) . ';--accent-soft:' . $tint( $light, '.1' ) . ';--on-accent:' . $on_light;
	$css .= ';--table-head:' . vl_rgb_hex( vl_hsl_rgb( $h, min( $s, 0.7 ), 0.22 ) ) . ';--table-head-2:' . vl_rgb_hex( vl_hsl_rgb( $h, min( $s, 0.7 ), 0.29 ) ) . ';--table-zebra:' . $tint( $light, '.06' ) . ';--table-hover:' . $tint( $light, '.1' ) . '}';
	$css .= 'html[data-theme="dark"]{--accent:' . vl_rgb_hex( $dark ) . ';--accent2:' . vl_rgb_hex( vl_hsl_rgb( $dark_hsl[0], $dark_hsl[1], min( 1, $dark_hsl[2] + 0.12 ) ) ) . ';--accent-soft:' . $tint( $dark, '.12' ) . ';--on-accent:' . $on_dark;
	$css .= ';--table-head:' . vl_rgb_hex( vl_hsl_rgb( $h, min( $s, 0.55 ), 0.17 ) ) . ';--table-head-2:' . vl_rgb_hex( vl_hsl_rgb( $h, min( $s, 0.55 ), 0.21 ) ) . ';--table-zebra:' . $tint( $dark, '.04' ) . ';--table-hover:' . $tint( $dark, '.09' ) . '}';
	return $css;
}

/** The accent colour in the block editor too, so links, quotes and tables look as on the site. */
function vl_editor_accent( $settings ) {
	$css = vl_accent_css();
	if ( '' !== $css ) {
		$settings['styles']   = isset( $settings['styles'] ) ? $settings['styles'] : array();
		$settings['styles'][] = array( 'css' => explode( 'html[data-theme="dark"]', $css )[0] );
	}
	return $settings;
}
add_filter( 'block_editor_settings_all', 'vl_editor_accent' );

/** The start page's sections that can be turned off in the Customizer, with their labels. */
function vl_home_sections() {
	return array(
		'tiles'        => __( 'Show the photo tiles', 'voyagelight' ),
		'stories'      => __( 'Show the latest stories', 'voyagelight' ),
		'journeys'     => __( 'Show the journeys', 'voyagelight' ),
		'destinations' => __( 'Show the featured destinations', 'voyagelight' ),
		'map'          => __( 'Show the travel map', 'voyagelight' ),
		'collections'  => __( 'Show the collections band', 'voyagelight' ),
	);
}

/** Whether the start page shows section `$key`. */
function vl_home_shows( $key ) {
	return (bool) get_theme_mod( 'vl_show_' . $key, true );
}

/** A number of posts for the start page, from 1 to 12. */
function vl_sanitize_count( $value ) {
	return max( 1, min( 12, absint( $value ) ) );
}

/** How many posts a list shows: `$key` is stories or journeys (start page), related (after a story) or footer (the footer's Journeys). */
function vl_home_count( $key, $fallback ) {
	return vl_sanitize_count( get_theme_mod( 'vl_' . $key . '_count', $fallback ) );
}

/** The start page's sections by name, for the Customizer's list that orders them. */
function vl_home_section_names() {
	return array(
		'tiles'        => __( 'Photo tiles', 'voyagelight' ),
		'stories'      => __( 'Latest stories', 'voyagelight' ),
		'journeys'     => __( 'Journeys', 'voyagelight' ),
		'destinations' => __( 'Featured destinations', 'voyagelight' ),
		'map'          => __( 'Travel map', 'voyagelight' ),
		'collections'  => __( 'Collections band', 'voyagelight' ),
	);
}

/**
 * The start page's section keys in order: those in `$saved` (comma-separated, by default the
 * Customizer's setting) that are known, once each, then any it leaves out in the theme's order.
 */
function vl_home_order( $saved = null ) {
	if ( null === $saved ) {
		$saved = get_theme_mod( 'vl_home_order', '' );
	}
	$known = array_keys( vl_home_sections() );
	$order = array_intersect( array_unique( array_map( 'trim', explode( ',', (string) $saved ) ) ), $known );
	return array_values( array_unique( array_merge( $order, $known ) ) );
}

/** A saved section order, kept to the known keys. */
function vl_sanitize_home_order( $value ) {
	return implode( ',', vl_home_order( $value ) );
}

/** A card summary's length in words, from 5 to 80. */
function vl_sanitize_summary_words( $value ) {
	return max( 5, min( 80, absint( $value ) ) );
}

/** A card's summary: the excerpt cut to the Customizer's length. */
function vl_card_summary( $post_id ) {
	return wp_trim_words( get_the_excerpt( $post_id ), vl_sanitize_summary_words( get_theme_mod( 'vl_summary_words', 20 ) ) );
}

/** The Customizer's list for ordering the start page, and its look. */
function vl_customize_scripts() {
	wp_enqueue_script( 'vl-customize-order', get_template_directory_uri() . '/assets/js/customize-order.js', array( 'customize-controls' ), VL_VERSION, true );
	wp_add_inline_style( 'customize-controls', '.vl-order{margin:6px 0 0;padding:0;list-style:none}.vl-order li{display:flex;align-items:center;gap:4px;margin:0 0 4px;padding:4px 4px 4px 10px;background:#fff;border:1px solid #dcdcde;border-radius:4px}.vl-order-name{flex:1}.vl-order li:first-child .vl-order-up,.vl-order li:last-child .vl-order-down{opacity:.35}' );
}
add_action( 'customize_controls_enqueue_scripts', 'vl_customize_scripts' );

/** A theme mod, or `$fallback` while it is empty. */
function vl_mod( $name, $fallback ) {
	$value = get_theme_mod( $name, '' );
	return '' === $value ? $fallback : $value;
}
