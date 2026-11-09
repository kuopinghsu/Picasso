<?php
/**
 * The start page's destinations section; front-page.php draws the sections in the order set in the Customizer.
 *
 * @package VoyageLight
 */
?>
<section id="destinations" class="shell section destination-section">
	<div class="section-heading">
		<div>
			<span class="eyebrow"><?php esc_html_e( 'Destinations & discovery', 'voyagelight' ); ?></span>
			<h2><?php esc_html_e( 'Featured destinations', 'voyagelight' ); ?></h2>
			<p><?php esc_html_e( 'Explore stories by country and region, then see where each place is on a map.', 'voyagelight' ); ?></p>
		</div>
	</div>
	<?php
	// The site's Destinations, as named in Posts > Destinations, or the theme's samples until there are three.
	// As many columns as tiles, up to five (three on a tablet), so a short row has no gap at its end.
	$destinations = vl_destination_items();
	$count        = count( $destinations );
	?>
	<div class="destination-grid<?php echo $count < 5 ? ' is-few' : ''; ?>" style="--cols:<?php echo (int) min( 5, max( 1, $count ) ); ?>;--cols-md:<?php echo (int) min( 3, max( 1, $count ) ); ?>">
		<?php
		foreach ( $destinations as $destination ) {
			echo '<a class="destination-card" href="' . esc_url( $destination[3] ) . '"><img src="' . esc_url( $destination[0] ) . '" alt="" loading="lazy">';
			echo '<span><b>' . esc_html( $destination[1] ) . '</b><small>' . esc_html( $destination[2] ) . '</small></span></a>';
		}
		?>
	</div>
</section>
