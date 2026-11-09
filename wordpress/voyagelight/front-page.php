<?php
get_header();
$stories_url = vl_stories_url();
$journeys_url = get_post_type_archive_link( 'vl_trip' );
// A link to a section of this page, or to `$elsewhere` when that section is turned off in the Customizer.
$to_section = function ( $section, $id, $elsewhere ) {
	return vl_home_shows( $section ) ? '#' . $id : $elsewhere;
};
$nav_cards = array(
	array( 'stories', __( 'Travel Stories', 'voyagelight' ), __( 'People, places & memories', 'voyagelight' ), $to_section( 'stories', 'stories', $stories_url ) ),
	array( 'destinations', __( 'Destinations', 'voyagelight' ), __( 'Browse by destination', 'voyagelight' ), $to_section( 'destinations', 'destinations', $stories_url ) ),
	array( 'journeys', __( 'Journeys', 'voyagelight' ), __( 'Day-by-day trip journals', 'voyagelight' ), $to_section( 'journeys', 'journeys', $journeys_url ) ),
	array( 'travel-map', __( 'Travel Map', 'voyagelight' ), __( 'Explore places on a map', 'voyagelight' ), $to_section( 'map', 'travel-map', $stories_url ) ),
);
// Each tile goes to its own part of this page, or to a page of its own: a tile whose section is
// turned off, and so would only repeat the stories link, is left out.
$nav_cards = array_values(
	array_filter(
		$nav_cards,
		function ( $card ) use ( $stories_url ) {
			return 'stories' === $card[0] || $card[3] !== $stories_url;
		}
	)
);
?>
<main id="main">
<section class="hero vl-photo-hero">
	<?php echo vl_hero_image(); // phpcs:ignore WordPress.Security.EscapeOutput.OutputNotEscaped -- escaped in vl_hero_image(). ?>
	<div class="shell hero-inner">
		<div class="hero-content">
			<span class="hero-kicker"><?php esc_html_e( 'Travel · explore · share', 'voyagelight' ); ?></span>
			<h1><?php echo esc_html( vl_mod( 'vl_hero_title', __( 'The world, through our journeys', 'voyagelight' ) ) ); ?></h1>
			<p><?php echo esc_html( vl_mod( 'vl_hero_subtitle', __( 'Stories, places and moments worth sharing.', 'voyagelight' ) ) ); ?></p>
			<div class="hero-buttons">
				<a class="button primary" href="<?php echo esc_url( $to_section( 'journeys', 'journeys', $journeys_url ) ); ?>"><?php esc_html_e( 'Explore journeys', 'voyagelight' ); ?> →</a>
				<a class="button ghost" href="<?php echo esc_url( $to_section( 'stories', 'stories', $stories_url ) ); ?>"><?php esc_html_e( 'Read travel stories', 'voyagelight' ); ?></a>
			</div>
		</div>
		<?php if ( '' === get_theme_mod( 'vl_hero_image', '' ) ) : ?>
			<span class="hero-caption"><?php esc_html_e( 'Norwegian fjords · Demo photography', 'voyagelight' ); ?></span>
		<?php endif; ?>
	</div>
</section>

<?php
// The sections, in the order set in Appearance > Customize > VoyageLight homepage; each is in template-parts/.
$home_parts = array(
	'to_section'   => $to_section,
	'stories_url'  => $stories_url,
	'journeys_url' => $journeys_url,
	'nav_cards'    => $nav_cards,
);
foreach ( vl_home_order() as $home_section ) {
	if ( vl_home_shows( $home_section ) ) {
		get_template_part( 'template-parts/home', $home_section, $home_parts );
	}
}
?>
</main>
<?php
get_footer();
