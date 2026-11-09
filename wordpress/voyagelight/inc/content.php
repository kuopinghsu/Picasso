<?php
/**
 * Story cards, photo tiles, Destinations, the category menus and panels, and the widget areas.
 *
 * @package VoyageLight
 */

if ( ! defined( 'ABSPATH' ) ) {
	exit;
}

/** The address of all stories: the posts page, or the post list when the site has none. */
function vl_stories_url() {
	return get_option( 'page_for_posts' ) ? get_permalink( get_option( 'page_for_posts' ) ) : home_url( '/?post_type=post' );
}

/** A map pin in a tinted circle, for the map placeholders and the empty atlas. */
function vl_pin_icon() {
	return '<span class="vl-pin" aria-hidden="true"><svg viewBox="0 0 24 24" width="24" height="24" focusable="false"><path d="M12 21s-7-6.2-7-11.5a7 7 0 0 1 14 0C19 14.8 12 21 12 21z" fill="none" stroke="currentColor" stroke-width="2" stroke-linejoin="round"/><circle cx="12" cy="9.5" r="2.5" fill="currentColor"/></svg></span>';
}

/** URL of a bundled demo photo; unknown names give the hero photo. */
function vl_photo( $slug = 'hero-norway' ) {
	$allowed = array( 'hero-norway', 'stories', 'destinations', 'journeys', 'travel-map', 'food-culture', 'guides' );
	if ( ! in_array( $slug, $allowed, true ) ) {
		$slug = 'hero-norway';
	}
	return get_template_directory_uri() . '/assets/images/photos/' . $slug . '.jpg';
}

/**
 * The illustration on the card of a post with no featured image: one of the six, chosen by the
 * post's ID, so a row of such cards doesn't repeat one picture and each card keeps its own.
 */
function vl_card_art( $post_id ) {
	$art = array( 'arctic', 'nordic', 'greenland', 'japan', 'spain', 'baltic' );
	return vl_image( $art[ absint( $post_id ) % count( $art ) ] );
}

/** URL of a bundled illustration; unknown names give the Arctic one. */
function vl_image( $slug = 'arctic' ) {
	$allowed = array( 'arctic', 'baltic', 'greenland', 'japan', 'nordic', 'spain' );
	if ( ! in_array( $slug, $allowed, true ) ) {
		$slug = 'arctic';
	}
	return get_template_directory_uri() . '/assets/images/' . $slug . '.svg';
}

/**
 * The photo tiles under the start page's hero, as array( image URL, name, note, link ) each.
 * Once the site has three top-level categories with stories, the tiles are those categories,
 * most stories first, each with the featured image of its latest story that has one;
 * until then `$defaults`, the theme's sections, as array( photo, name, note, link ).
 */
function vl_photo_nav_items( $defaults ) {
	$children = vl_category_children();
	$skip     = (int) get_option( 'default_category' );
	$terms    = array();
	foreach ( isset( $children[0] ) ? $children[0] : array() as $term ) {
		if ( $term->term_id !== $skip && ! is_wp_error( get_term_link( $term ) ) ) {
			$terms[] = $term;
		}
	}
	if ( count( $terms ) < 3 ) {
		$items = array();
		foreach ( $defaults as $item ) {
			$items[] = array( vl_photo( $item[0] ), $item[1], $item[2], $item[3] );
		}
		return $items;
	}
	usort(
		$terms,
		function ( $a, $b ) {
			return $b->count - $a->count ?: strcmp( $a->name, $b->name );
		}
	);
	$photos = array( 'stories', 'destinations', 'journeys', 'travel-map', 'food-culture', 'guides' );
	$items  = array();
	$used   = array();
	foreach ( array_slice( $terms, 0, 6 ) as $i => $term ) {
		$image = vl_fresh_photo( array( 'cat' => $term->term_id ), $used );
		/* translators: %d: number of stories in a category. */
		$note    = sprintf( _n( '%d story', '%d stories', $term->count, 'voyagelight' ), $term->count );
		$items[] = array( $image ? $image : vl_photo( $photos[ $i ] ), $term->name, $note, get_term_link( $term ) );
	}
	return $items;
}

/**
 * The card-size featured image of the latest published story matching `$query` (get_posts()
 * arguments) whose image isn't in `$used` yet, so tiles side by side don't show one photo twice;
 * the image found is added to `$used`. '' when every such story's image is taken or there's none,
 * and the tile then shows the theme's own picture. Password-protected stories are left out.
 */
function vl_fresh_photo( $query, &$used ) {
	$posts = get_posts(
		array_merge(
			array(
				'post_type'           => 'post',
				'post_status'         => 'publish',
				'posts_per_page'      => 6,
				'fields'              => 'ids',
				'no_found_rows'       => true,
				'ignore_sticky_posts' => true,
				'has_password'        => false,
				'meta_key'            => '_thumbnail_id', // phpcs:ignore WordPress.DB.SlowDBQuery -- a few posts per tile, on the start page only.
				'orderby'             => array(
					'date' => 'DESC',
					'ID'   => 'DESC',
				),
			),
			$query
		)
	);
	foreach ( $posts as $post_id ) {
		$image = (int) get_post_thumbnail_id( $post_id );
		if ( $image && ! in_array( $image, $used, true ) ) {
			$url = wp_get_attachment_image_url( $image, 'vl-card' );
			if ( $url ) {
				$used[] = $image;
				return $url;
			}
		}
	}
	return '';
}

/**
 * The start page's "Featured destinations" tiles, as array( image URL, name, note, link ) each.
 * Once three Destinations have stories of their own, the tiles are the five with the most,
 * named as in Posts > Destinations, with the term's description as the note (or its number
 * of stories) and the featured image of its latest story that has one. Until then, the theme's
 * five sample places, linked to the Destination with the same slug once it exists.
 */
function vl_destination_items() {
	$art   = array( 'nordic', 'greenland', 'baltic', 'spain', 'japan', 'arctic' );
	$terms = get_terms(
		array(
			'taxonomy'   => 'destination',
			'hide_empty' => true,
			'orderby'    => 'count',
			'order'      => 'DESC',
			'number'     => 5,
		)
	);
	$terms = is_wp_error( $terms ) ? array() : $terms;
	if ( count( $terms ) < 3 ) {
		$items = array();
		foreach ( array( array( 'nordic', 'Nordic', '北歐' ), array( 'greenland', 'Greenland', '格陵蘭' ), array( 'baltic', 'Baltic', '波羅的海' ), array( 'spain', 'Spain', '西班牙' ), array( 'japan', 'Japan', '日本' ) ) as $sample ) {
			$term    = get_term_by( 'slug', $sample[0], 'destination' );
			$url     = $term ? get_term_link( $term ) : vl_stories_url();
			$items[] = array( vl_image( $sample[0] ), $sample[1], $sample[2], is_wp_error( $url ) ? vl_stories_url() : $url );
		}
		return $items;
	}
	$items = array();
	$used  = array();
	foreach ( $terms as $i => $term ) {
		$url = get_term_link( $term );
		if ( is_wp_error( $url ) ) {
			continue;
		}
		$image = vl_fresh_photo( array( 'tax_query' => array( array( 'taxonomy' => 'destination', 'terms' => $term->term_id ) ) ), $used ); // phpcs:ignore WordPress.DB.SlowDBQuery -- a few posts per tile, on the start page only.
		if ( ! $image ) {
			$image = vl_image( in_array( $term->slug, $art, true ) ? $term->slug : $art[ $i % count( $art ) ] );
		}
		$note = trim( wp_strip_all_tags( $term->description ) );
		if ( '' === $note ) {
			/* translators: %d: number of stories in a category. */
			$note = sprintf( _n( '%d story', '%d stories', $term->count, 'voyagelight' ), $term->count );
		}
		$items[] = array( $image, $term->name, wp_html_excerpt( $note, 40, '…' ), $url );
	}
	return $items;
}

/** Minutes to read `$content`: words at 220 a minute, CJK characters at 400. */
function vl_read_minutes( $content ) {
	$text  = wp_strip_all_tags( $content );
	$cjk   = preg_match_all( '/[\x{3040}-\x{30FF}\x{3400}-\x{9FFF}\x{AC00}-\x{D7AF}]/u', $text );
	$words = str_word_count( preg_replace( '/[^\x00-\x7F]+/', ' ', $text ) );
	return max( 1, (int) ceil( $words / 220 + $cjk / 400 ) );
}

/** A story's card; `$lead` makes it the large first card of the start page's latest stories. */
function vl_card( $post_id, $lead = false ) {
	$destinations = get_the_terms( $post_id, 'destination' );
	$destination  = ( $destinations && ! is_wp_error( $destinations ) ) ? $destinations[0]->name : __( 'Travel journal', 'voyagelight' );
	$categories   = get_the_category( $post_id );
	$category     = $categories ? $categories[0]->name : __( 'Story', 'voyagelight' );
	$link         = get_permalink( $post_id );
	// The title is the card's one link; CSS stretches it over the whole card.
	echo '<article class="story-card' . ( $lead ? ' is-lead' : '' ) . '"><div class="story-visual">';
	if ( has_post_thumbnail( $post_id ) ) {
		echo get_the_post_thumbnail( $post_id, 'vl-card', array( 'loading' => 'lazy', 'alt' => '' ) );
	} else {
		echo '<img src="' . esc_url( vl_card_art( $post_id ) ) . '" alt="" loading="lazy">';
	}
	echo '<span class="story-badge">' . esc_html( $destination ) . '</span></div>';
	echo '<div class="story-copy"><span class="eyebrow">' . esc_html( $category ) . ' · ' . esc_html( get_the_date( '', $post_id ) ) . '</span>';
	echo '<h3><a href="' . esc_url( $link ) . '">' . esc_html( get_the_title( $post_id ) ) . '</a></h3>';
	echo '<p>' . esc_html( vl_card_summary( $post_id ) ) . '</p></div></article>';
}

/**
 * Sidebar category list as a nested tree, always expanded: each sub-category
 * (Europe > 2025-spain) is indented under its parent, with its post count in
 * parentheses. Counts include the children's posts and empty categories are left out.
 */
function vl_category_tree() {
	return vl_category_branch( vl_category_children(), 0, is_category() ? (int) get_queried_object_id() : 0 );
}

/** Categories that have posts (counting their children's), keyed by parent id, sorted by name. */
function vl_category_children() {
	static $children = null;
	if ( null !== $children ) {
		return $children;
	}
	$children = array();
	$terms    = get_terms(
		array(
			'taxonomy'   => 'category',
			'hide_empty' => false,
			'pad_counts' => true,
			'orderby'    => 'name',
		)
	);
	foreach ( is_wp_error( $terms ) ? array() : $terms as $term ) {
		if ( $term->count > 0 ) {
			$children[ $term->parent ][] = $term;
		}
	}
	return $children;
}

/** Ids of the current category, or the current post's categories, and all their ancestors. */
function vl_category_current() {
	$current = array();
	if ( is_category() ) {
		$current[] = (int) get_queried_object_id();
	} elseif ( is_singular( 'post' ) ) {
		$current = wp_list_pluck( get_the_category(), 'term_id' );
	}
	$ids = array();
	foreach ( $current as $id ) {
		$ids = array_merge( $ids, array( (int) $id ), get_ancestors( $id, 'category', 'taxonomy' ) );
	}
	return array_map( 'intval', $ids );
}

/**
 * Header menu items for the categories, like a travel blog's region menu: each top-level
 * category (Europe, Asia) is an item and its sub-categories (2025-spain) its dropdown.
 * Only two levels are shown; the default "Uncategorized" category is left out.
 */
function vl_category_menu_items() {
	$children = vl_category_children();
	$current  = vl_category_current();
	$skip     = (int) get_option( 'default_category' );
	$html     = '';
	foreach ( isset( $children[0] ) ? $children[0] : array() as $term ) {
		if ( $term->term_id === $skip ) {
			continue;
		}
		$subs    = isset( $children[ $term->term_id ] ) ? $children[ $term->term_id ] : array();
		$classes = array( 'menu-item' );
		if ( $subs ) {
			$classes[] = 'menu-item-has-children';
		}
		if ( in_array( $term->term_id, $current, true ) ) {
			$classes[] = 'current-menu-ancestor';
		}
		$html .= '<li class="' . esc_attr( implode( ' ', $classes ) ) . '"><a href="' . esc_url( get_term_link( $term ) ) . '"' . vl_aria_current( $term ) . '>' . esc_html( $term->name ) . '</a>';
		if ( $subs ) {
			$html .= vl_sub_toggle( $term->name ) . '<ul class="sub-menu">';
			foreach ( $subs as $sub ) {
				$here  = in_array( $sub->term_id, $current, true );
				$html .= '<li class="menu-item' . ( $here ? ' current-menu-item' : '' ) . '"><a href="' . esc_url( get_term_link( $sub ) ) . '"' . vl_aria_current( $sub ) . '>' . esc_html( $sub->name ) . '</a></li>';
			}
			$html .= '</ul>';
		}
		$html .= '</li>';
	}
	return $html;
}

/** ` aria-current="page"` for a link to the category being shown, as WordPress's own menus mark it. */
function vl_aria_current( $term ) {
	return is_category( $term->term_id ) ? ' aria-current="page"' : '';
}

/**
 * The button after a top-level menu item that opens its sub-menu. Touch screens have no hover,
 * and keyboards reach the sub-menu through it; theme.js opens and closes it.
 */
function vl_sub_toggle( $name ) {
	/* translators: %s: name of a menu item, such as Europe. */
	$label = sprintf( __( 'Show the %s menu', 'voyagelight' ), $name );
	return '<button class="sub-toggle" type="button" aria-expanded="false"><span class="screen-reader-text">' . esc_html( $label ) . '</span>'
		. '<svg viewBox="0 0 24 24" width="14" height="14" aria-hidden="true" focusable="false"><path d="M6 9l6 6 6-6" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"/></svg></button>';
}

/** The same button in a header menu made in Appearance > Menus. */
function vl_nav_sub_toggle( $output, $item, $depth, $args ) {
	if ( 0 === $depth && isset( $args->theme_location ) && 'primary' === $args->theme_location && in_array( 'menu-item-has-children', (array) $item->classes, true ) ) {
		$output .= vl_sub_toggle( wp_strip_all_tags( $item->title ) );
	}
	return $output;
}
add_filter( 'walker_nav_menu_start_el', 'vl_nav_sub_toggle', 10, 4 );

/** The "Travel categories" panel used by the sidebar and the start page; empty when there are none. */
function vl_category_panel() {
	$tree = vl_category_tree();
	if ( '' === $tree ) {
		return '';
	}
	return '<div class="sidebar-panel"><h2>' . esc_html__( 'Travel categories', 'voyagelight' ) . '</h2><ul class="cat-tree">' . $tree . '</ul></div>';
}

/**
 * The list of one of the theme's sidebar panels: 'categories', 'destinations', 'types' or 'journeys'.
 * '' when the panel has nothing to list (no categories with stories, no published Journeys).
 */
function vl_panel_list( $panel ) {
	if ( 'categories' === $panel ) {
		$tree = vl_category_tree();
		return '' === $tree ? '' : '<ul class="cat-tree">' . $tree . '</ul>';
	}
	if ( 'destinations' === $panel || 'types' === $panel ) {
		$items = wp_list_categories(
			array(
				'taxonomy'         => 'destinations' === $panel ? 'destination' : 'travel_type',
				'title_li'         => '',
				'hide_empty'       => false,
				'echo'             => false,
				// Nothing, not WordPress's "No categories", so a panel with no terms is left out.
				'show_option_none' => '',
			)
		);
		return '' === trim( (string) $items ) ? '' : '<ul>' . $items . '</ul>';
	}
	$journeys = get_posts(
		array(
			'post_type'      => 'vl_trip',
			'post_status'    => 'publish',
			'posts_per_page' => -1,
			'no_found_rows'  => true,
			'orderby'        => array(
				'title' => 'ASC',
				'ID'    => 'ASC',
			),
		)
	);
	$items = '';
	foreach ( $journeys as $journey ) {
		$items .= '<li><a href="' . esc_url( get_permalink( $journey ) ) . '">' . esc_html( get_the_title( $journey ) ) . '</a></li>';
	}
	return '' === $items ? '' : '<ul>' . $items . '</ul>';
}

/** The theme's sidebar panels as one widget, so they can sit among WordPress's own in Appearance > Widgets. */
class VL_Panel_Widget extends WP_Widget {
	public function __construct() {
		parent::__construct(
			'vl_panel',
			__( 'VoyageLight panel', 'voyagelight' ),
			array(
				'description'           => __( 'One of the theme’s own panels: travel categories, destinations, place types or journeys.', 'voyagelight' ),
				'show_instance_in_rest' => true,
			)
		);
	}

	/** The panels, keyed as vl_panel_list() takes them, with their headings. */
	public static function panels() {
		return array(
			'categories'   => __( 'Travel categories', 'voyagelight' ),
			'destinations' => __( 'Destinations', 'voyagelight' ),
			'types'        => __( 'Place types', 'voyagelight' ),
			'journeys'     => __( 'Journeys', 'voyagelight' ),
		);
	}

	public function widget( $args, $instance ) {
		$panels = self::panels();
		$panel  = isset( $instance['panel'], $panels[ $instance['panel'] ] ) ? $instance['panel'] : 'categories';
		$list   = vl_panel_list( $panel );
		if ( '' !== $list ) {
			echo $args['before_widget'] . $args['before_title'] . esc_html( $panels[ $panel ] ) . $args['after_title'] . $list . $args['after_widget']; // phpcs:ignore WordPress.Security.EscapeOutput.OutputNotEscaped -- the wrappers come from register_sidebar(), the list is escaped in vl_panel_list().
		}
	}

	public function form( $instance ) {
		$panel = isset( $instance['panel'] ) ? $instance['panel'] : 'categories';
		echo '<p><label for="' . esc_attr( $this->get_field_id( 'panel' ) ) . '">' . esc_html__( 'Panel', 'voyagelight' ) . '</label>';
		echo '<select class="widefat" id="' . esc_attr( $this->get_field_id( 'panel' ) ) . '" name="' . esc_attr( $this->get_field_name( 'panel' ) ) . '">';
		foreach ( self::panels() as $key => $label ) {
			echo '<option value="' . esc_attr( $key ) . '"' . selected( $panel, $key, false ) . '>' . esc_html( $label ) . '</option>';
		}
		echo '</select></p>';
		return '';
	}

	public function update( $new_instance, $old_instance ) {
		$panel = isset( $new_instance['panel'] ) ? (string) $new_instance['panel'] : '';
		return array( 'panel' => isset( self::panels()[ $panel ] ) ? $panel : 'categories' );
	}
}

/**
 * Widget areas for the sidebar and the footer. Each shows the theme's own content until a widget
 * is added to it in Appearance > Widgets.
 */
function vl_widgets_init() {
	register_sidebar(
		array(
			'name'          => __( 'Sidebar', 'voyagelight' ),
			'id'            => 'sidebar-1',
			'description'   => __( 'Beside stories and archives. Left empty, the theme’s own panels are shown.', 'voyagelight' ),
			'before_widget' => '<div id="%1$s" class="sidebar-panel widget %2$s">',
			'after_widget'  => '</div>',
			'before_title'  => '<h2 class="widget-title">',
			'after_title'   => '</h2>',
		)
	);
	register_sidebar(
		array(
			'name'          => __( 'Footer', 'voyagelight' ),
			'id'            => 'footer-1',
			'description'   => __( 'Columns beside the site’s name in the footer. Left empty, the top categories and the latest Journeys are shown.', 'voyagelight' ),
			'before_widget' => '<div id="%1$s" class="footer-col widget %2$s">',
			'after_widget'  => '</div>',
			'before_title'  => '<h2 class="widget-title">',
			'after_title'   => '</h2>',
		)
	);
	register_sidebar(
		array(
			'name'          => __( 'Start page sidebar', 'voyagelight' ),
			'id'            => 'home-1',
			'description'   => __( 'Beside the latest stories on the start page. Left empty, the travel categories are shown.', 'voyagelight' ),
			'before_widget' => '<div id="%1$s" class="sidebar-panel widget %2$s">',
			'after_widget'  => '</div>',
			'before_title'  => '<h2 class="widget-title">',
			'after_title'   => '</h2>',
		)
	);
	register_widget( 'VL_Panel_Widget' );
}
add_action( 'widgets_init', 'vl_widgets_init' );

/**
 * Whether block widget `$id` is one WordPress adds when it's installed, unchanged: Search, or
 * Recent Posts, Recent Comments, Archives or Categories under their heading (in any language).
 */
function vl_is_install_widget( $id, $blocks ) {
	if ( ! preg_match( '/^block-([2-6])$/', $id, $match ) || ! isset( $blocks[ $match[1] ]['content'] ) ) {
		return false;
	}
	$content = trim( preg_replace( '/\s+/', ' ', $blocks[ $match[1] ]['content'] ) );
	return '<!-- wp:search /-->' === $content
		|| (bool) preg_match( '~^<!-- wp:group --> ?<div class="wp-block-group"> ?<!-- wp:heading --> ?<h2[^>]*>[^<]*</h2> ?<!-- /wp:heading --> ?<!-- wp:(latest-posts|latest-comments|archives|categories)( \{[^}]*\})? /--> ?</div> ?<!-- /wp:group -->$~', $content );
}

/**
 * On a new site WordPress fills the first widget area with its own Search, Recent Posts and
 * other widgets, which would hide the theme's category tree. When a VoyageLight area holds only
 * those, unchanged, they move to Appearance > Widgets > Inactive widgets, from where they can be
 * put back; an area with any other widget is left as it is. Done once after the theme is turned on.
 */
function vl_park_install_widgets() {
	if ( get_option( 'vl_install_widgets_checked' ) ) {
		return;
	}
	update_option( 'vl_install_widgets_checked', 1 );
	$areas  = wp_get_sidebars_widgets();
	$blocks = get_option( 'widget_block', array() );
	$moved  = false;
	foreach ( array( 'sidebar-1', 'footer-1', 'home-1' ) as $area ) {
		$widgets = isset( $areas[ $area ] ) ? (array) $areas[ $area ] : array();
		if ( ! $widgets ) {
			continue;
		}
		foreach ( $widgets as $id ) {
			if ( ! vl_is_install_widget( $id, $blocks ) ) {
				continue 2;
			}
		}
		$areas['wp_inactive_widgets'] = array_merge( isset( $areas['wp_inactive_widgets'] ) ? (array) $areas['wp_inactive_widgets'] : array(), $widgets );
		$areas[ $area ]               = array();
		$moved                        = true;
	}
	if ( $moved ) {
		wp_set_sidebars_widgets( $areas );
	}
}
// After WordPress has moved the widgets over from the previous theme, which it does at init.
add_action( 'wp_loaded', 'vl_park_install_widgets' );

/** Checks again the next time the theme is turned on. */
function vl_reset_install_widgets() {
	delete_option( 'vl_install_widgets_checked' );
}
add_action( 'switch_theme', 'vl_reset_install_widgets' );

/** The `<li>`s for the categories under `$parent`, each with its own children nested inside. */
function vl_category_branch( $children, $parent, $current ) {
	$html = '';
	foreach ( isset( $children[ $parent ] ) ? $children[ $parent ] : array() as $term ) {
		// The count is inside the link, so the start page can show each category as one chip.
		$html .= '<li class="cat-item"><a href="' . esc_url( get_term_link( $term ) ) . '"' . ( $term->term_id === $current ? ' aria-current="page"' : '' ) . '>' . esc_html( $term->name )
			. ' <span class="cat-count">(' . esc_html( number_format_i18n( $term->count ) ) . ')</span></a>';
		if ( ! empty( $children[ $term->term_id ] ) ) {
			$html .= '<ul class="children">' . vl_category_branch( $children, $term->term_id, $current ) . '</ul>';
		}
		$html .= '</li>';
	}
	return $html;
}

function vl_body_class( $classes ) {
	$classes[] = 'vl-site';
	return $classes;
}
add_filter( 'body_class', 'vl_body_class' );
