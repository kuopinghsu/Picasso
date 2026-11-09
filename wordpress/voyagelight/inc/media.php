<?php
/**
 * Camera details under photos, Table block widths and YouTube start times.
 *
 * @package VoyageLight
 */

if ( ! defined( 'ABSPATH' ) ) {
	exit;
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

/**
 * Gives a Table block the column widths chosen in Picasso, which keeps them
 * in percent in its own attribute, as a <colgroup>.
 */
function vl_table_widths( $content, $block ) {
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
add_filter( 'render_block_core/table', 'vl_table_widths', 10, 2 );

/** Keep Picasso's column widths when the block editor saves a table. */
function vl_table_width_attributes( $args, $block_type ) {
	if ( 'core/table' === $block_type ) {
		$args['attributes']['picassoColumnWidths'] = array( 'type' => 'array' );
	}
	return $args;
}
add_filter( 'register_block_type_args', 'vl_table_width_attributes', 10, 2 );

/**
 * Starts an embedded YouTube video where its address says, such as t=90s
 * or t=1m30s: the player YouTube gives for an address leaves that out.
 */
function vl_youtube_start( $html, $url ) {
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
add_filter( 'embed_oembed_html', 'vl_youtube_start', 10, 2 );
