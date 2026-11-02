<?php
get_header();
// Quotes, brackets and backslashes are percent-encoded so the address can't end CSS's url().
$hero      = str_replace( array( '"', "'", '(', ')', '\\' ), array( '%22', '%27', '%28', '%29', '%5C' ), esc_url_raw( vl_hero_url() ) );
$nav_cards = array(
	array( 'stories', __( 'Travel Stories', 'voyagelight' ), __( 'People, places & memories', 'voyagelight' ), '#stories' ),
	array( 'destinations', __( 'Destinations', 'voyagelight' ), __( 'Browse by destination', 'voyagelight' ), '#destinations' ),
	array( 'journeys', __( 'Journeys', 'voyagelight' ), __( 'Day-by-day trip journals', 'voyagelight' ), '#journeys' ),
	array( 'travel-map', __( 'Travel Map', 'voyagelight' ), __( 'Explore places on a map', 'voyagelight' ), '#travel-map' ),
	array( 'food-culture', __( 'Food & Culture', 'voyagelight' ), __( 'Local flavours & culture', 'voyagelight' ), home_url( '/category/food-culture/' ) ),
	array( 'guides', __( 'Travel Guides', 'voyagelight' ), __( 'Plan with useful notes', 'voyagelight' ), home_url( '/category/travel-guides/' ) ),
);
$stories_url = get_option( 'page_for_posts' ) ? get_permalink( get_option( 'page_for_posts' ) ) : home_url( '/?post_type=post' );
?>
<main id="main">
<section class="hero vl-photo-hero" style="--hero-image:url('<?php echo esc_attr( $hero ); ?>')">
	<div class="shell hero-inner">
		<div class="hero-content">
			<span class="hero-kicker"><?php esc_html_e( 'TRAVEL · EXPLORE · SHARE', 'voyagelight' ); ?></span>
			<h1><?php echo esc_html( vl_mod( 'vl_hero_title', __( 'The world, through our journeys', 'voyagelight' ) ) ); ?></h1>
			<p><?php echo esc_html( vl_mod( 'vl_hero_subtitle', __( 'Stories, places and moments worth sharing.', 'voyagelight' ) ) ); ?></p>
			<div class="hero-buttons">
				<a class="button primary" href="#journeys"><?php esc_html_e( 'Explore journeys', 'voyagelight' ); ?> →</a>
				<a class="button ghost" href="#stories"><?php esc_html_e( 'Read travel stories', 'voyagelight' ); ?></a>
			</div>
		</div>
		<?php if ( '' === get_theme_mod( 'vl_hero_image', '' ) ) : ?>
			<span class="hero-caption"><?php esc_html_e( 'Norwegian fjords · Demo photography', 'voyagelight' ); ?></span>
		<?php endif; ?>
	</div>
</section>

<section class="shell vl-photo-nav" aria-label="<?php esc_attr_e( 'Explore travel sections', 'voyagelight' ); ?>">
	<?php foreach ( $nav_cards as $item ) : ?>
		<a class="vl-photo-tile" href="<?php echo esc_url( $item[3] ); ?>">
			<img src="<?php echo esc_url( vl_photo( $item[0] ) ); ?>" alt="" loading="lazy" decoding="async">
			<span class="vl-photo-tile-content"><strong><?php echo esc_html( $item[1] ); ?></strong><small><?php echo esc_html( $item[2] ); ?></small></span>
		</a>
	<?php endforeach; ?>
</section>

<section id="stories" class="shell section">
	<div class="section-heading">
		<div>
			<span class="eyebrow"><?php esc_html_e( 'TRAVEL JOURNAL', 'voyagelight' ); ?></span>
			<h2><?php esc_html_e( 'Latest travel stories', 'voyagelight' ); ?></h2>
			<p><?php esc_html_e( 'Places, people, and the moments worth remembering.', 'voyagelight' ); ?></p>
		</div>
		<a class="section-more" href="<?php echo esc_url( $stories_url ); ?>"><?php esc_html_e( 'All stories ↗', 'voyagelight' ); ?></a>
	</div>
	<div class="story-grid">
		<?php
		$latest = new WP_Query(
			array(
				'posts_per_page'      => 5,
				'ignore_sticky_posts' => true,
				'orderby'             => array(
					'date' => 'DESC',
					'ID'   => 'DESC',
				),
			)
		);
		if ( $latest->have_posts() ) {
			while ( $latest->have_posts() ) {
				$latest->the_post();
				vl_card( get_the_ID() );
			}
			wp_reset_postdata();
		} else {
			$samples = array(
				array( 'greenland', __( 'Eqi Glacier: A Close Encounter with the Icefjord', 'voyagelight' ), __( 'Greenland', 'voyagelight' ), __( 'Travel stories', 'voyagelight' ) ),
				array( 'nordic', __( 'Under the Northern Lights', 'voyagelight' ), __( 'Nordic', 'voyagelight' ), __( 'Nature & Scenery', 'voyagelight' ) ),
				array( 'spain', __( 'Toledo: A City of History and Light', 'voyagelight' ), __( 'Spain', 'voyagelight' ), __( 'Culture', 'voyagelight' ) ),
				array( 'japan', __( 'Tokyo: A Walk Through Spring', 'voyagelight' ), __( 'Japan', 'voyagelight' ), __( 'City Walks', 'voyagelight' ) ),
				array( 'baltic', __( 'A Morning in the Baltic Old Town', 'voyagelight' ), __( 'Baltic', 'voyagelight' ), __( 'History & Culture', 'voyagelight' ) ),
			);
			foreach ( $samples as $sample ) {
				echo '<article class="story-card"><div class="story-visual"><img src="' . esc_url( vl_image( $sample[0] ) ) . '" alt="' . esc_attr( $sample[1] ) . '"><span class="story-badge">' . esc_html( $sample[2] ) . '</span></div>';
				echo '<div class="story-copy"><span class="eyebrow">' . esc_html( $sample[3] ) . ' · ' . esc_html__( 'SAMPLE', 'voyagelight' ) . '</span><h3>' . esc_html( $sample[1] ) . '</h3>';
				echo '<p>' . esc_html__( 'Demo illustration. Publish your own story to replace this sample.', 'voyagelight' ) . '</p></div></article>';
			}
		}
		?>
	</div>
</section>

<section id="journeys" class="shell section">
	<div class="section-heading">
		<div>
			<span class="eyebrow"><?php esc_html_e( 'ONE JOURNEY · MANY STORIES', 'voyagelight' ); ?></span>
			<h2><?php esc_html_e( 'Explore journeys', 'voyagelight' ); ?></h2>
			<p><?php esc_html_e( 'Follow each trip day by day, with stops and stories together on one map.', 'voyagelight' ); ?></p>
		</div>
		<a class="section-more" href="<?php echo esc_url( get_post_type_archive_link( 'vl_trip' ) ); ?>"><?php esc_html_e( 'All journeys ↗', 'voyagelight' ); ?></a>
	</div>
	<div class="trip-grid">
		<?php
		$trips = get_posts(
			array(
				'post_type'      => 'vl_trip',
				'post_status'    => 'publish',
				'posts_per_page' => 3,
				'orderby'        => array(
					'date' => 'DESC',
					'ID'   => 'DESC',
				),
			)
		);
		if ( $trips ) {
			foreach ( $trips as $trip ) {
				vl_trip_card( $trip->ID );
			}
		} else {
			echo '<div class="map-empty">' . esc_html__( 'Publish your first Journey from the WordPress dashboard to show it here.', 'voyagelight' ) . '</div>';
		}
		?>
	</div>
</section>

<section id="destinations" class="shell section destination-section">
	<div class="section-heading">
		<div>
			<span class="eyebrow"><?php esc_html_e( 'DESTINATIONS & DISCOVERY', 'voyagelight' ); ?></span>
			<h2><?php esc_html_e( 'Featured destinations', 'voyagelight' ); ?></h2>
			<p><?php esc_html_e( 'Explore stories by country and region, then see where each place is on a map.', 'voyagelight' ); ?></p>
		</div>
	</div>
	<div class="destination-grid">
		<?php
		// The tiles name each place in English and Chinese whatever the site's
		// language, and link to the Destination with that slug once it exists.
		$destinations = array(
			array( 'nordic', 'Nordic', '北歐' ),
			array( 'greenland', 'Greenland', '格陵蘭' ),
			array( 'baltic', 'Baltic', '波羅的海' ),
			array( 'spain', 'Spain', '西班牙' ),
			array( 'japan', 'Japan', '日本' ),
		);
		foreach ( $destinations as $destination ) {
			$term = get_term_by( 'slug', $destination[0], 'destination' );
			$url  = $term ? get_term_link( $term ) : '#stories';
			echo '<a class="destination-card" href="' . esc_url( is_wp_error( $url ) ? '#stories' : $url ) . '"><img src="' . esc_url( vl_image( $destination[0] ) ) . '" alt="">';
			echo '<span><b>' . esc_html( $destination[1] ) . '</b><small>' . esc_html( $destination[2] ) . '</small></span></a>';
		}
		?>
	</div>
</section>

<section id="travel-map" class="shell section">
	<div class="section-heading">
		<div>
			<span class="eyebrow"><?php esc_html_e( 'PLACE & CONTEXT', 'voyagelight' ); ?></span>
			<h2><?php esc_html_e( 'Explore the travel map', 'voyagelight' ); ?></h2>
			<p><?php esc_html_e( 'See where places are, and how they relate to nearby destinations.', 'voyagelight' ); ?></p>
		</div>
	</div>
	<?php
	$atlas = vl_map_data();
	if ( $atlas ) {
		echo vl_render_map( $atlas, __( 'Travel atlas', 'voyagelight' ) ); // phpcs:ignore WordPress.Security.EscapeOutput -- escaped inside.
	} else {
		echo '<div class="map-empty"><span>⌖</span><div><strong>' . esc_html__( 'Your travel atlas starts here.', 'voyagelight' ) . '</strong>';
		echo '<p>' . esc_html__( 'Give published travel posts a latitude and longitude to populate this map automatically.', 'voyagelight' ) . '</p></div></div>';
	}
	?>
</section>

<section class="shell section editorial-grid">
	<div class="editorial-panel">
		<span class="eyebrow"><?php esc_html_e( 'COLLECTIONS', 'voyagelight' ); ?></span>
		<h2><?php esc_html_e( 'Every place has a story.', 'voyagelight' ); ?></h2>
		<p><?php esc_html_e( 'Follow the route, the local culture, and the little details along the way. Build a travel journal with your own voice.', 'voyagelight' ); ?></p>
		<a href="#stories" class="text-link"><?php esc_html_e( 'Explore the journal ↗', 'voyagelight' ); ?></a>
	</div>
	<div class="editorial-photo"><img src="<?php echo esc_url( vl_image( 'japan' ) ); ?>" alt="<?php esc_attr_e( 'Illustrated cherry blossom scene', 'voyagelight' ); ?>"></div>
</section>
</main>
<?php
get_footer();
