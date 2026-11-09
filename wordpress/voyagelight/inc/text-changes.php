<?php
/**
 * The theme's words as changed in Appearance > Customize > VoyageLight text.
 *
 * @package VoyageLight
 */

if ( ! defined( 'ABSPATH' ) ) {
	exit;
}

/**
 * The changes from Appearance > Customize > VoyageLight text: each text as the site shows it,
 * in English or in its translation, and what to show instead. Read once WordPress has loaded,
 * so the Customizer's preview sees the changes being typed.
 */
function vl_text_changes() {
	static $changes = null;
	if ( null !== $changes ) {
		return $changes;
	}
	$found = array();
	foreach ( array_slice( preg_split( '/\r\n|\r|\n/', (string) get_theme_mod( 'vl_text_changes', '' ) ), 0, 300 ) as $line ) {
		$parts = explode( '=', $line, 2 );
		// Nothing after the equals sign hides the text.
		if ( 2 === count( $parts ) && '' !== trim( $parts[0] ) ) {
			$found[ vl_text_key( $parts[0] ) ] = trim( $parts[1] );
		}
	}
	if ( did_action( 'wp_loaded' ) ) {
		$changes = $found;
	}
	return $found;
}

/**
 * How a text is looked up in the changes: trimmed and in lower case, so a change typed as the
 * site shows it matches even where the stylesheet capitalises it (TRAVEL JOURNAL for "Travel journal").
 */
function vl_text_key( $text ) {
	$text = trim( $text );
	return function_exists( 'mb_strtolower' ) ? mb_strtolower( $text, 'UTF-8' ) : strtolower( $text );
}

/** The sorted sprintf placeholders, or null if a format contains an unsupported percent sequence. */
function vl_placeholders( $text, $strict = false ) {
	preg_match_all( '/%%|%(?:[1-9]\d*\$)?[sdfF]|%/', $text, $found );
	if ( $strict && in_array( '%', $found[0], true ) ) {
		return null;
	}
	$placeholders = array_diff( $found[0], array( '%%', '%' ) );
	sort( $placeholders );
	return $placeholders;
}

/**
 * `$translation` as changed in the Customizer, when a change names it or its English `$originals`.
 * A change that drops or adds a placeholder is ignored, so it can't break the text it is filled into.
 */
function vl_change_text( $translation, $originals ) {
	$changes = vl_text_changes();
	if ( ! $changes ) {
		return $translation;
	}
	foreach ( array_merge( array( $translation ), $originals ) as $text ) {
		$key = vl_text_key( $text );
		if ( isset( $changes[ $key ] ) ) {
			$expected = vl_placeholders( $originals[0] );
			return vl_placeholders( $changes[ $key ], ! empty( $expected ) ) === $expected ? $changes[ $key ] : $translation;
		}
	}
	return $translation;
}
add_filter(
	'gettext_voyagelight',
	function ( $translation, $text ) {
		return vl_change_text( $translation, array( $text ) );
	},
	10,
	2
);
add_filter(
	'gettext_with_context_voyagelight',
	function ( $translation, $text ) {
		return vl_change_text( $translation, array( $text ) );
	},
	10,
	2
);
add_filter(
	'ngettext_voyagelight',
	function ( $translation, $single, $plural ) {
		return vl_change_text( $translation, array( $single, $plural ) );
	},
	10,
	3
);
