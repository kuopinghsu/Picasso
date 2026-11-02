<?php
/**
 * Checks the theme's location reader, and the Picasso Places plugin's copy of
 * it in the plugin's namespace, against geo-cases.json, the cases geo-test.js
 * checks the JavaScript reader against. Loading both also shows they can.
 *
 *     php wordpress/tests/geo-test.php
 */

require __DIR__ . '/../voyagelight/inc/geo.php';
require __DIR__ . '/../picasso-places/inc/geo.php';

$cases  = json_decode( file_get_contents( __DIR__ . '/geo-cases.json' ), true );
$failed = 0;
$passed = 0;

function vl_test_same( $expected, $actual ) {
	if ( is_array( $expected ) && is_array( $actual ) ) {
		if ( array_keys( $expected ) !== array_keys( $actual ) ) {
			return false;
		}
		foreach ( $expected as $key => $value ) {
			if ( ! vl_test_same( $value, $actual[ $key ] ) ) {
				return false;
			}
		}
		return true;
	}
	if ( is_float( $expected ) || is_float( $actual ) ) {
		return is_numeric( $expected ) && is_numeric( $actual ) && abs( $expected - $actual ) < 2e-6;
	}
	return $expected === $actual;
}

function vl_test( $name, $input, $expected, $actual ) {
	global $failed, $passed;
	if ( vl_test_same( $expected, $actual ) ) {
		++$passed;
		return;
	}
	++$failed;
	fwrite( STDERR, "FAIL $name " . json_encode( $input, JSON_UNESCAPED_UNICODE ) . "\n  expected " . json_encode( $expected ) . "\n  got      " . json_encode( $actual ) . "\n" );
}

// The theme's functions, then the plugin's of the same names.
foreach ( array( 'theme' => '', 'plugin' => 'PicassoPlaces\\' ) as $reader => $ns ) {
	foreach ( $cases['location'] as $case ) {
		vl_test( "$reader location", $case[0], $case[1], ( $ns . 'vl_geo_read' )( $case[0] ) );
		vl_test( "$reader vl_parse_location", $case[0], is_array( $case[1] ) ? $case[1] : null, ( $ns . 'vl_parse_location' )( $case[0] ) );
	}
	foreach ( $cases['stop'] as $case ) {
		vl_test( "$reader stop", $case[0], $case[1], ( $ns . 'vl_geo_stop_line' )( $case[0] ) );
	}
	foreach ( $cases['day'] as $case ) {
		vl_test( "$reader day", $case[0], $case[1], ( $ns . 'vl_geo_day_line' )( $case[0] ) );
	}
	foreach ( $cases['problems'] as $case ) {
		vl_test( "$reader problems", $case[0], $case[2], ( $ns . 'vl_geo_problems' )( $case[0], $case[1], 200 ) );
	}
	vl_test( "$reader unescape", '39.8628%2C-4.0273 100%25', '39.8628,-4.0273 100%25', ( $ns . 'vl_geo_unescape' )( '39.8628%2C-4.0273 100%25' ) );
	vl_test( "$reader line limit", "a\nb\nc", array( 'a', 'b' ), ( $ns . 'vl_geo_lines' )( "a\nb\nc", 2 ) );
	vl_test( "$reader valid coord", '90.5', null, ( $ns . 'vl_valid_coord' )( '90.5', 'lat' ) );
	vl_test( "$reader valid coord", '-180', -180.0, ( $ns . 'vl_valid_coord' )( '-180', 'lon' ) );
}

echo "$passed passed, $failed failed\n";
exit( $failed ? 1 : 0 );
