<?php
/**
 * Reading the places authors type or paste: coordinates in decimal degrees or
 * degrees, minutes and seconds, and links from Google Maps, Apple Maps,
 * OpenStreetMap and geo: addresses. Plain PHP without WordPress, so the tests
 * can run it alone. assets/js/geo.js reads the same way; change both together.
 *
 * @package VoyageLight
 */

// Coordinates are saved only when the author types them. Photo EXIF GPS is never read.
// A Journey day can run past a year. assets/js/geo.js DAY_MAX is the same number.
const VL_GEO_DAY_MAX = 9999;
function vl_valid_coord( $value, $kind ) {
	if ( ! is_scalar( $value ) || ! is_numeric( $value ) ) {
		return null;
	}
	$number = (float) $value;
	$limit  = 'lat' === $kind ? 90 : 180;
	return is_finite( $number ) && $number >= -$limit && $number <= $limit ? $number : null;
}

function vl_geo_is_number( $text ) {
	return 1 === preg_match( '/^[+-]?\d+(?:\.\d+)?$/', $text );
}

/** Decodes the printable ASCII characters that map links percent-encode, such as %2C for a comma. */
function vl_geo_unescape( $text ) {
	return preg_replace_callback(
		'/%([2-7][0-9a-f])/i',
		function ( $match ) {
			$char = chr( hexdec( $match[1] ) );
			return '%' === $char || "\x7f" === $char ? $match[0] : $char;
		},
		(string) $text
	);
}

/** The first `$max` lines of `$text`. */
function vl_geo_lines( $text, $max ) {
	return array_slice( preg_split( '/\r\n|\r|\n/', (string) $text ), 0, $max );
}

/**
 * Latitude and longitude from text such as 39.8628, -4.0273 or
 * 39°51'46.1"N 4°01'38.3"W, before any range check; null when unreadable.
 */
function vl_geo_text_pair( $text ) {
	$text = strtr( $text, array( '′' => "'", '’' => "'", '‘' => "'", '´' => "'", '″' => '"', '”' => '"', '“' => '"', 'º' => '°', '˚' => '°', '，' => ',', '；' => ';' ) );
	$text = strtoupper( str_replace( "''", '"', $text ) );
	$core = '([+-]?)\s*([0-9]+(?:\.[0-9]+)?)(?:\s*°)?(?:\s*([0-9]+(?:\.[0-9]+)?)\s*\')?(?:\s*([0-9]+(?:\.[0-9]+)?)\s*")?';
	$sep  = '(?:\s*[,;]\s*|\s+|(?<![0-9.])|(?![0-9.]))';
	if ( preg_match( '/^' . $core . $sep . $core . '$/u', $text, $m ) ) {
		$first  = vl_geo_dms( $m[1], $m[2], $m[3] ?? '', $m[4] ?? '', '' );
		$second = vl_geo_dms( $m[5], $m[6], $m[7] ?? '', $m[8] ?? '', '' );
		return null === $first || null === $second ? null : array( $first, $second );
	}
	if ( preg_match( '/^([NSEW])\s*' . $core . $sep . '([NSEW])\s*' . $core . '$/u', $text, $m ) ) {
		$parts = array( array( $m[1], $m[2], $m[3], $m[4] ?? '', $m[5] ?? '' ), array( $m[6], $m[7], $m[8], $m[9] ?? '', $m[10] ?? '' ) );
	} elseif ( preg_match( '/^' . $core . '\s*([NSEW])' . $sep . $core . '\s*([NSEW])$/u', $text, $m ) ) {
		$parts = array( array( $m[5], $m[1], $m[2], $m[3], $m[4] ), array( $m[10], $m[6], $m[7], $m[8], $m[9] ) );
	} else {
		return null;
	}
	$values = array();
	foreach ( $parts as $part ) {
		$values[ $part[0] ] = vl_geo_dms( $part[1], $part[2], $part[3], $part[4], $part[0] );
	}
	$lat = $values['N'] ?? ( $values['S'] ?? null );
	$lon = $values['E'] ?? ( $values['W'] ?? null );
	return null === $lat || null === $lon || 2 !== count( $values ) ? null : array( $lat, $lon );
}

/** Degrees, minutes and seconds as decimal degrees, negative for S, W or a minus sign; null when malformed. */
function vl_geo_dms( $sign, $degrees, $minutes, $seconds, $hemisphere ) {
	if ( ( '' !== $minutes && false !== strpos( $degrees, '.' ) ) || ( '' !== $seconds && ( '' === $minutes || false !== strpos( $minutes, '.' ) ) )
		|| (float) $minutes >= 60 || (float) $seconds >= 60 || ( '-' === $sign && '' !== $hemisphere ) ) {
		return null;
	}
	$value = (float) $degrees + (float) $minutes / 60 + (float) $seconds / 3600;
	return '-' === $sign || 'S' === $hemisphere || 'W' === $hemisphere ? -$value : $value;
}

/** Latitude and longitude from a map link, before any range check; null when it has none. */
function vl_geo_link_pair( $text ) {
	$n    = '([+-]?\d+(?:\.\d+)?)';
	$pair = $n . '\s*,\s*\+?\s*' . $n;
	if ( preg_match( '/!3d' . $n . '!4d' . $n . '/', $text, $m ) ) {
		return array( (float) $m[1], (float) $m[2] );
	}
	if ( preg_match( '/[?&#]mlat=' . $n . '/i', $text, $a ) && preg_match( '/[?&#]mlon=' . $n . '/i', $text, $b ) ) {
		return array( (float) $a[1], (float) $b[1] );
	}
	$patterns = array(
		'/@' . $pair . '/',
		'/[?&](?:q|query)=(?:loc:)?\s*' . $pair . '/i',
		'/[?&](?:ll|coordinate|center)=' . $pair . '/i',
		'~/maps/(?:search|place)/' . $pair . '~i',
		'~#map=\d+(?:\.\d+)?/' . $n . '/' . $n . '~',
		'/^geo:' . $pair . '/i',
	);
	foreach ( $patterns as $pattern ) {
		if ( preg_match( $pattern, $text, $m ) ) {
			return array( (float) $m[1], (float) $m[2] );
		}
	}
	if ( preg_match( '/[?&](?:q|query)=([^&#]+)/i', $text, $m ) ) {
		return vl_geo_text_pair( trim( str_replace( '+', ' ', $m[1] ) ) );
	}
	return null;
}

/**
 * Reads a location: array( latitude, longitude ) rounded to six decimals, or
 * why it can't be read: 'empty', 'short-link', 'link', 'range' or 'unreadable'.
 */
function vl_geo_read( $text ) {
	$text = trim( str_replace( array( "\u{a0}", "\u{202f}" ), ' ', vl_geo_unescape( $text ) ) );
	if ( '' === $text ) {
		return 'empty';
	}
	if ( strlen( $text ) > 2048 ) {
		return 'unreadable';
	}
	if ( preg_match( '~^(?:https?://)?(?:www\.)?(?:maps\.app\.goo\.gl|goo\.gl|g\.co|g\.page|apple\.co|maps\.apple)/|^(?:https?://)?(?:www\.)?(?:osm|openstreetmap)\.org/go/~i', $text ) ) {
		return 'short-link';
	}
	if ( preg_match( '~^(?:[a-z][a-z0-9+.-]*:|www\.)|\b(?:maps|openstreetmap|osm)\.[a-z]~i', $text ) ) {
		$pair = vl_geo_link_pair( $text );
		if ( null === $pair ) {
			return 'link';
		}
	} else {
		$pair = vl_geo_text_pair( $text );
		if ( null === $pair ) {
			return 'unreadable';
		}
	}
	$lat = vl_valid_coord( $pair[0], 'lat' );
	$lon = vl_valid_coord( $pair[1], 'lon' );
	if ( null === $lat || null === $lon ) {
		return 'range';
	}
	return array( round( $lat, 6 ), round( $lon, 6 ) );
}

/**
 * A latitude and longitude as `lat, lon` text that vl_geo_read() reads back:
 * six decimals at most, never in exponent form such as 1.0E-5. assets/js/geo.js format() matches.
 */
function vl_geo_format( $lat, $lon ) {
	$number = function ( $value ) {
		$text = rtrim( rtrim( sprintf( '%.6F', (float) $value ), '0' ), '.' );
		return '-0' === $text ? '0' : $text;
	};
	return $number( $lat ) . ', ' . $number( $lon );
}

/** array( latitude, longitude ) from any form vl_geo_read() accepts, or null. */
function vl_parse_location( $text ) {
	$point = vl_geo_read( $text );
	return is_array( $point ) ? $point : null;
}

/**
 * A route stop from `Place | location | note`, or the older
 * `Place | latitude | longitude | note`: array( name, lat, lon, note ),
 * why it can't be read ('name' or a vl_geo_read() reason), or null for a blank line.
 */
function vl_geo_stop_line( $line ) {
	$line = trim( (string) $line );
	if ( '' === $line ) {
		return null;
	}
	$cols = array_map( 'trim', explode( '|', $line ) );
	if ( count( $cols ) >= 3 && vl_geo_is_number( $cols[1] ) && vl_geo_is_number( $cols[2] ) ) {
		$where = $cols[1] . ', ' . $cols[2];
		$note  = implode( ' | ', array_slice( $cols, 3 ) );
	} else {
		$where = $cols[1] ?? '';
		$note  = implode( ' | ', array_slice( $cols, 2 ) );
	}
	if ( '' === $cols[0] ) {
		return 'name';
	}
	$point = vl_geo_read( $where );
	if ( ! is_array( $point ) ) {
		return $point;
	}
	return array( 'name' => $cols[0], 'lat' => $point[0], 'lon' => $point[1], 'note' => $note );
}

/**
 * A Journey stop from `Day | Place | location | note | post ID`, or the older
 * `Day | Place | latitude | longitude | note | post ID`: array( day, name, lat,
 * lon, note, post_id ), why it can't be read ('day', 'name' or a vl_geo_read()
 * reason), or null for a blank line.
 */
function vl_geo_day_line( $line ) {
	$line = trim( (string) $line );
	if ( '' === $line ) {
		return null;
	}
	$cols = array_pad( array_map( 'trim', explode( '|', $line ) ), 3, '' );
	if ( count( $cols ) >= 4 && vl_geo_is_number( $cols[2] ) && vl_geo_is_number( $cols[3] ) ) {
		$where = $cols[2] . ', ' . $cols[3];
		$rest  = array_slice( $cols, 4 );
	} else {
		$where = $cols[2];
		$rest  = array_slice( $cols, 3 );
	}
	if ( ! preg_match( '/^\d+$/', $cols[0] ) || (int) $cols[0] < 1 || (int) $cols[0] > VL_GEO_DAY_MAX ) {
		return 'day';
	}
	if ( '' === $cols[1] ) {
		return 'name';
	}
	$point = vl_geo_read( $where );
	if ( ! is_array( $point ) ) {
		return $point;
	}
	// A note can hold a "|", so the post ID is the last column after it, when that is a number or empty.
	$post = '';
	if ( count( $rest ) >= 2 && preg_match( '/^\d*$/', end( $rest ) ) ) {
		$post = array_pop( $rest );
	}
	return array(
		'day'     => (int) $cols[0],
		'name'    => $cols[1],
		'lat'     => $point[0],
		'lon'     => $point[1],
		'note'    => implode( ' | ', $rest ),
		'post_id' => preg_match( '/^\d+$/', $post ) ? (int) $post : 0,
	);
}

/** The lines of `$text` that can't be read: array( line number, text, reason ) each. */
function vl_geo_problems( $text, $kind, $max_lines ) {
	$problems = array();
	foreach ( vl_geo_lines( $text, $max_lines ) as $i => $line ) {
		$row = 'day' === $kind ? vl_geo_day_line( $line ) : vl_geo_stop_line( $line );
		if ( is_string( $row ) ) {
			$problems[] = array( $i + 1, trim( $line ), $row );
		}
	}
	return $problems;
}
