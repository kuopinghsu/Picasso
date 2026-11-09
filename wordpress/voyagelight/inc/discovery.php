<?php
/**
 * Being found and shared: the address bases of Journeys, Destinations and Place types in
 * Settings > Permalinks, breadcrumbs, related stories, link previews for social apps
 * (Open Graph) and structured data for search engines (JSON-LD). A password-protected post
 * shares only its title, never its summary, photo or places.
 *
 * @package VoyageLight
 */

if ( ! defined( 'ABSPATH' ) ) {
	exit;
}

/* ---------- Address bases ---------- */

/** The address bases that Settings > Permalinks can change, with their defaults. */
function vl_permalink_defaults() {
	return array(
		'trip'        => 'trips',
		'destination' => 'destination',
		'place_type'  => 'place-type',
	);
}

/** The address base of 'trip', 'destination' or 'place_type', as set in Settings > Permalinks or else its default. */
function vl_permalink_base( $key ) {
	$value    = (string) get_option( 'vl_' . $key . '_base', '' );
	$defaults = vl_permalink_defaults();
	return '' !== $value ? $value : $defaults[ $key ];
}

/** An address base as typed: latin letters, digits and hyphens only; '' for the default. */
function vl_clean_base( $text ) {
	$slug = strtolower( preg_replace( '/[^A-Za-z0-9]+/', '-', remove_accents( (string) $text ) ) );
	return trim( $slug, '-' );
}

/** The three fields, in the Optional section of Settings > Permalinks beside the category and tag bases. */
function vl_permalink_settings() {
	$labels = array(
		'trip'        => __( 'Journey base', 'voyagelight' ),
		'destination' => __( 'Destination base', 'voyagelight' ),
		'place_type'  => __( 'Place type base', 'voyagelight' ),
	);
	foreach ( vl_permalink_defaults() as $key => $default ) {
		add_settings_field(
			'vl_' . $key . '_base',
			$labels[ $key ],
			function () use ( $key, $default ) {
				echo '<input name="vl_' . esc_attr( $key ) . '_base" id="vl_' . esc_attr( $key ) . '_base" type="text" class="regular-text code" value="' . esc_attr( get_option( 'vl_' . $key . '_base', '' ) ) . '" placeholder="' . esc_attr( $default ) . '">';
				echo '<p class="description">' . esc_html__( 'Latin letters, digits and hyphens, such as journeys. Leave it empty for the theme’s own.', 'voyagelight' ) . '</p>';
			},
			'permalink',
			'optional',
			array( 'label_for' => 'vl_' . $key . '_base' )
		);
	}
}
add_action( 'admin_init', 'vl_permalink_settings' );

/** Saves the bases with the rest of Settings > Permalinks, and has the address rules rebuilt on the next request. */
function vl_permalink_save() {
	if ( ! isset( $_POST['vl_trip_base'] ) || ! current_user_can( 'manage_options' ) ) {
		return;
	}
	check_admin_referer( 'update-permalink' );
	$changed = false;
	foreach ( array_keys( vl_permalink_defaults() ) as $key ) {
		$field = 'vl_' . $key . '_base';
		$slug  = isset( $_POST[ $field ] ) ? vl_clean_base( sanitize_text_field( wp_unslash( $_POST[ $field ] ) ) ) : '';
		if ( (string) get_option( $field, '' ) !== $slug ) {
			update_option( $field, $slug );
			$changed = true;
		}
	}
	if ( $changed ) {
		update_option( 'vl_flush_rewrites', 1 );
	}
}
add_action( 'load-options-permalink.php', 'vl_permalink_save' );

/** Rebuilds the address rules once the post type and taxonomies are registered with their new bases. */
function vl_flush_rewrites() {
	if ( get_option( 'vl_flush_rewrites' ) ) {
		delete_option( 'vl_flush_rewrites' );
		flush_rewrite_rules( false );
	}
}
add_action( 'init', 'vl_flush_rewrites', 99 );

/* ---------- Breadcrumbs ---------- */

/**
 * The trail to the page being shown, as array( name, URL ) from the home page to the page itself,
 * or array() on the start page, search results and the 404 page. A story's trail goes through its
 * deepest category, leaving out the default "Uncategorized".
 */
function vl_breadcrumb_items() {
	$items = array( array( __( 'Home', 'voyagelight' ), home_url( '/' ) ) );
	$chain = function ( $term ) use ( &$items ) {
		foreach ( array_reverse( get_ancestors( $term->term_id, $term->taxonomy, 'taxonomy' ) ) as $ancestor ) {
			$parent = get_term( $ancestor, $term->taxonomy );
			if ( $parent && ! is_wp_error( $parent ) ) {
				$items[] = array( $parent->name, get_term_link( $parent ) );
			}
		}
		$items[] = array( $term->name, get_term_link( $term ) );
	};
	$id = get_queried_object_id();
	if ( is_singular( 'post' ) ) {
		$skip       = (int) get_option( 'default_category' );
		$categories = array();
		foreach ( get_the_category( $id ) as $category ) {
			if ( (int) $category->term_id !== $skip ) {
				$categories[ count( get_ancestors( $category->term_id, 'category', 'taxonomy' ) ) * 100000 + count( $categories ) ] = $category;
			}
		}
		if ( $categories ) {
			krsort( $categories );
			$chain( reset( $categories ) );
		}
		$items[] = array( vl_plain_title( $id ), get_permalink( $id ) );
	} elseif ( is_singular( 'vl_trip' ) ) {
		$items[] = array( __( 'Journeys', 'voyagelight' ), get_post_type_archive_link( 'vl_trip' ) );
		$items[] = array( vl_plain_title( $id ), get_permalink( $id ) );
	} elseif ( is_page() && ! is_front_page() ) {
		foreach ( array_reverse( get_post_ancestors( $id ) ) as $ancestor ) {
			$items[] = array( vl_plain_title( $ancestor ), get_permalink( $ancestor ) );
		}
		$items[] = array( vl_plain_title( $id ), get_permalink( $id ) );
	} elseif ( is_category() || is_tag() || is_tax( array( 'destination', 'travel_type' ) ) ) {
		$chain( get_queried_object() );
	} elseif ( is_post_type_archive( 'vl_trip' ) ) {
		$items[] = array( __( 'Journeys', 'voyagelight' ), get_post_type_archive_link( 'vl_trip' ) );
	} else {
		return array();
	}
	return $items;
}

/** The breadcrumb trail above a page's title, unless turned off in the Customizer. */
function vl_breadcrumbs() {
	$items = get_theme_mod( 'vl_show_breadcrumbs', true ) ? vl_breadcrumb_items() : array();
	if ( count( $items ) < 2 ) {
		return '';
	}
	$last = count( $items ) - 1;
	$out  = '<nav class="breadcrumbs" aria-label="' . esc_attr__( 'Breadcrumb', 'voyagelight' ) . '"><ol>';
	foreach ( $items as $i => $item ) {
		if ( $i === $last || is_wp_error( $item[1] ) || ! $item[1] ) {
			$out .= '<li><span' . ( $i === $last ? ' aria-current="page"' : '' ) . '>' . esc_html( $item[0] ) . '</span></li>';
		} else {
			$out .= '<li><a href="' . esc_url( $item[1] ) . '">' . esc_html( $item[0] ) . '</a></li>';
		}
	}
	return $out . '</ol></nav>';
}

/* ---------- Related stories ---------- */

/**
 * Up to `$count` published stories near story `$post_id`, newest first: those in the same
 * Destination, then those in the same categories (not "Uncategorized"). Password-protected ones are left out.
 */
function vl_related_posts( $post_id, $count = 3 ) {
	$found   = array();
	$exclude = array( (int) $post_id );
	$by      = array(
		'destination' => wp_get_post_terms( $post_id, 'destination', array( 'fields' => 'ids' ) ),
		'category'    => array_diff( wp_get_post_categories( $post_id ), array( (int) get_option( 'default_category' ) ) ),
	);
	foreach ( $by as $taxonomy => $terms ) {
		if ( count( $found ) >= $count || is_wp_error( $terms ) || ! $terms ) {
			continue;
		}
		$ids     = get_posts(
			array(
				'post_type'           => 'post',
				'post_status'         => 'publish',
				'posts_per_page'      => $count - count( $found ),
				'post__not_in'        => $exclude, // phpcs:ignore WordPressVIPMinimum.Performance.WPQueryParams -- a handful of posts.
				'has_password'        => false,
				'no_found_rows'       => true,
				'ignore_sticky_posts' => true,
				'fields'              => 'ids',
				'orderby'             => array(
					'date' => 'DESC',
					'ID'   => 'DESC',
				),
				'tax_query'           => array( array( 'taxonomy' => $taxonomy, 'terms' => array_map( 'intval', array_values( $terms ) ) ) ), // phpcs:ignore WordPress.DB.SlowDBQuery -- three posts at the end of a story.
			)
		);
		$found   = array_merge( $found, $ids );
		$exclude = array_merge( $exclude, $ids );
	}
	return $found;
}

/* ---------- Link previews and structured data ---------- */

/** Whether an SEO plugin is active, which writes its own previews and structured data; the theme then writes none. */
function vl_seo_plugin_active() {
	return defined( 'WPSEO_VERSION' ) || defined( 'RANK_MATH_VERSION' ) || defined( 'AIOSEO_VERSION' ) || defined( 'SEOPRESS_VERSION' ) || function_exists( 'the_seo_framework' );
}

/** A story or Journey's summary for previews: its excerpt, or the start of its text. */
function vl_summary( $post_id ) {
	$post = get_post( $post_id );
	$text = has_excerpt( $post ) ? $post->post_excerpt : excerpt_remove_blocks( strip_shortcodes( $post->post_content ) );
	$text = html_entity_decode( wp_strip_all_tags( $text ), ENT_QUOTES, 'UTF-8' );
	return wp_html_excerpt( trim( preg_replace( '/\s+/u', ' ', $text ) ), 200, '…' );
}

/**
 * What the page being shown shares: site, title, description, url, type ('article' or 'website'),
 * image (array( url, width, height ) or null), and for a story or Journey its dates and author.
 */
function vl_share_data() {
	$data = array(
		'site'        => get_bloginfo( 'name' ),
		'title'       => get_bloginfo( 'name' ),
		'description' => get_bloginfo( 'description' ),
		'url'         => home_url( '/' ),
		'type'        => 'website',
		'image'       => null,
	);
	$id   = get_queried_object_id();
	if ( is_singular() && $id ) {
		$data['title'] = vl_plain_title( $id );
		$data['url']   = get_permalink( $id );
		if ( is_singular( array( 'post', 'vl_trip' ) ) ) {
			$data['type']      = 'article';
			$data['published'] = get_the_date( 'c', $id );
			$data['modified']  = get_the_modified_date( 'c', $id );
			$data['author']    = get_the_author_meta( 'display_name', (int) get_post_field( 'post_author', $id ) );
		}
		if ( '' === (string) get_post_field( 'post_password', $id ) ) {
			$summary = vl_summary( $id );
			if ( '' !== $summary ) {
				$data['description'] = $summary;
			}
			$photo = has_post_thumbnail( $id ) ? wp_get_attachment_image_src( get_post_thumbnail_id( $id ), 'full' ) : false;
			if ( $photo ) {
				$data['image'] = array( $photo[0], (int) $photo[1], (int) $photo[2] );
			}
		}
	} elseif ( is_category() || is_tag() || is_tax() ) {
		$term          = get_queried_object();
		$data['title'] = $term->name;
		$data['url']   = get_term_link( $term );
		$about         = trim( wp_strip_all_tags( term_description( $term ) ) );
		if ( '' !== $about ) {
			$data['description'] = wp_html_excerpt( $about, 200, '…' );
		}
	} elseif ( is_post_type_archive( 'vl_trip' ) ) {
		$data['title'] = __( 'Journeys', 'voyagelight' );
		$data['url']   = get_post_type_archive_link( 'vl_trip' );
	} elseif ( is_home() && ! is_front_page() ) {
		$data['title'] = get_the_title( (int) get_option( 'page_for_posts' ) );
		$data['url']   = vl_stories_url();
	}
	if ( is_wp_error( $data['url'] ) ) {
		$data['url'] = home_url( '/' );
	}
	if ( ! $data['image'] ) {
		$data['image'] = array( vl_hero_url(), 0, 0 );
	}
	return $data;
}

/** Link previews for Facebook, LINE, X and other apps, and the page's description for search engines. */
function vl_share_meta() {
	if ( vl_seo_plugin_active() || is_404() || is_search() ) {
		return;
	}
	$data = vl_share_data();
	$tags = array(
		'og:site_name'   => $data['site'],
		'og:locale'      => get_locale(),
		'og:type'        => $data['type'],
		'og:title'       => $data['title'],
		'og:description' => $data['description'],
		'og:url'         => $data['url'],
		'og:image'       => $data['image'][0],
	);
	if ( $data['image'][1] ) {
		$tags['og:image:width']  = $data['image'][1];
		$tags['og:image:height'] = $data['image'][2];
	}
	if ( 'article' === $data['type'] ) {
		$tags['article:published_time'] = $data['published'];
		$tags['article:modified_time']  = $data['modified'];
	}
	if ( '' !== $data['description'] ) {
		echo '<meta name="description" content="' . esc_attr( $data['description'] ) . '">' . "\n";
	}
	foreach ( $tags as $property => $content ) {
		if ( '' !== (string) $content ) {
			echo '<meta property="' . esc_attr( $property ) . '" content="' . esc_attr( $content ) . '">' . "\n";
		}
	}
	echo '<meta name="twitter:card" content="summary_large_image">' . "\n";
}
add_action( 'wp_head', 'vl_share_meta', 5 );

/**
 * Structured data for search engines: the website and its search on the start page; a story as a
 * BlogPosting with its places, or a Journey as a TouristTrip with its itinerary; and the breadcrumb trail.
 */
function vl_structured_data() {
	if ( vl_seo_plugin_active() || is_404() ) {
		return;
	}
	$graph = array();
	if ( is_front_page() ) {
		$graph[] = array(
			'@type'           => 'WebSite',
			'name'            => get_bloginfo( 'name' ),
			'description'     => get_bloginfo( 'description' ),
			'url'             => home_url( '/' ),
			'inLanguage'      => get_bloginfo( 'language' ),
			'potentialAction' => array(
				'@type'       => 'SearchAction',
				'target'      => home_url( '/?s={search_term_string}' ),
				'query-input' => 'required name=search_term_string',
			),
		);
	}
	$id = get_queried_object_id();
	if ( is_singular( array( 'post', 'vl_trip' ) ) && $id && '' === (string) get_post_field( 'post_password', $id ) ) {
		$data      = vl_share_data();
		$publisher = array(
			'@type' => 'Organization',
			'name'  => get_bloginfo( 'name' ),
			'url'   => home_url( '/' ),
		);
		$logo      = has_custom_logo() ? wp_get_attachment_image_url( (int) get_theme_mod( 'custom_logo' ), 'full' ) : '';
		if ( $logo ) {
			$publisher['logo'] = array(
				'@type' => 'ImageObject',
				'url'   => $logo,
			);
		}
		$places = array();
		foreach ( is_singular( 'vl_trip' ) ? vl_trip_points( $id ) : vl_post_points( $id ) as $point ) {
			$places[] = array(
				'@type' => 'Place',
				'name'  => $point['name'],
				'geo'   => array(
					'@type'     => 'GeoCoordinates',
					'latitude'  => $point['lat'],
					'longitude' => $point['lon'],
				),
			);
		}
		$item = array(
			'@type'            => is_singular( 'vl_trip' ) ? 'TouristTrip' : 'BlogPosting',
			'name'             => $data['title'],
			'description'      => $data['description'],
			'url'              => $data['url'],
			'mainEntityOfPage' => $data['url'],
			'image'            => array( $data['image'][0] ),
			'datePublished'    => $data['published'],
			'dateModified'     => $data['modified'],
			'inLanguage'       => get_bloginfo( 'language' ),
			'author'           => array(
				'@type' => 'Person',
				'name'  => $data['author'],
				'url'   => get_author_posts_url( (int) get_post_field( 'post_author', $id ) ),
			),
			'publisher'        => $publisher,
		);
		if ( is_singular( 'post' ) ) {
			$item['headline'] = wp_html_excerpt( $data['title'], 110, '…' );
			if ( $places ) {
				$item['contentLocation'] = array_slice( $places, 0, 20 );
			}
		} elseif ( $places ) {
			$list = array();
			foreach ( array_slice( $places, 0, 100 ) as $i => $place ) {
				$list[] = array(
					'@type'    => 'ListItem',
					'position' => $i + 1,
					'item'     => $place,
				);
			}
			$item['itinerary'] = array(
				'@type'           => 'ItemList',
				'numberOfItems'   => count( $list ),
				'itemListElement' => $list,
			);
		}
		$graph[] = $item;
	}
	$trail = vl_breadcrumb_items();
	if ( count( $trail ) > 1 ) {
		$list = array();
		foreach ( $trail as $i => $crumb ) {
			$entry = array(
				'@type'    => 'ListItem',
				'position' => $i + 1,
				'name'     => $crumb[0],
			);
			if ( $crumb[1] && ! is_wp_error( $crumb[1] ) ) {
				$entry['item'] = $crumb[1];
			}
			$list[] = $entry;
		}
		$graph[] = array(
			'@type'           => 'BreadcrumbList',
			'itemListElement' => $list,
		);
	}
	if ( $graph ) {
		echo '<script type="application/ld+json">' . wp_json_encode( array( '@context' => 'https://schema.org', '@graph' => $graph ), JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE | JSON_HEX_TAG | JSON_HEX_AMP ) . '</script>' . "\n";
	}
}
add_action( 'wp_head', 'vl_structured_data', 6 );
