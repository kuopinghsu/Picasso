<?php
get_header();
?>
<main id="main" class="shell section">
	<div class="section-heading"><div><?php echo vl_breadcrumbs(); // phpcs:ignore WordPress.Security.EscapeOutput.OutputNotEscaped -- escaped in vl_breadcrumbs(). ?><span class="eyebrow"><?php esc_html_e( 'Journey collections', 'voyagelight' ); ?></span><h1><?php esc_html_e( 'Journeys', 'voyagelight' ); ?></h1><p><?php esc_html_e( 'Explore full travel routes, day-by-day itineraries, maps and related stories.', 'voyagelight' ); ?></p></div></div>
	<div class="trip-grid">
		<?php
		if ( have_posts() ) {
			while ( have_posts() ) {
				the_post();
				vl_trip_card( get_the_ID() );
			}
		} else {
			echo '<p>' . esc_html__( 'No journeys published yet.', 'voyagelight' ) . '</p>';
		}
		?>
	</div>
	<?php the_posts_pagination(); ?>
</main>
<?php
get_footer();
