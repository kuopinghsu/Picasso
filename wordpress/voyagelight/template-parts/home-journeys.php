<?php
/**
 * The start page's journeys section; front-page.php draws the sections in the order set in the Customizer.
 *
 * @package VoyageLight
 */

$journeys_url = $args['journeys_url'];
?>
<section id="journeys" class="shell section">
	<div class="section-heading">
		<div>
			<span class="eyebrow"><?php esc_html_e( 'One journey · many stories', 'voyagelight' ); ?></span>
			<h2><?php esc_html_e( 'Explore journeys', 'voyagelight' ); ?></h2>
			<p><?php esc_html_e( 'Follow each trip day by day, with stops and stories together on one map.', 'voyagelight' ); ?></p>
		</div>
		<a class="section-more" href="<?php echo esc_url( $journeys_url ); ?>"><?php esc_html_e( 'All journeys ↗', 'voyagelight' ); ?></a>
	</div>
	<div class="trip-grid">
		<?php
		$trips = get_posts(
			array(
				'post_type'      => 'vl_trip',
				'post_status'    => 'publish',
				'posts_per_page' => vl_home_count( 'journeys', 3 ),
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
