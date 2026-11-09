<?php
/**
 * The start page's map section; front-page.php draws the sections in the order set in the Customizer.
 *
 * @package VoyageLight
 */
?>
<section id="travel-map" class="shell section">
	<div class="section-heading">
		<div>
			<span class="eyebrow"><?php esc_html_e( 'Place & context', 'voyagelight' ); ?></span>
			<h2><?php esc_html_e( 'Explore the travel map', 'voyagelight' ); ?></h2>
			<p><?php esc_html_e( 'See where places are, and how they relate to nearby destinations.', 'voyagelight' ); ?></p>
		</div>
	</div>
	<?php
	$atlas = vl_map_data();
	if ( $atlas ) {
		echo vl_render_map( $atlas, __( 'Travel atlas', 'voyagelight' ), false, false ); // phpcs:ignore WordPress.Security.EscapeOutput -- escaped inside.
	} else {
		echo '<div class="map-empty">' . vl_pin_icon() . '<div><strong>' . esc_html__( 'Your travel atlas starts here.', 'voyagelight' ) . '</strong>';
		echo '<p>' . esc_html__( 'Give published travel posts a latitude and longitude to populate this map automatically.', 'voyagelight' ) . '</p></div></div>';
	}
	?>
</section>
