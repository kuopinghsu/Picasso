<?php
/**
 * The start page's tiles section; front-page.php draws the sections in the order set in the Customizer.
 *
 * @package VoyageLight
 */

$nav_cards = $args['nav_cards'];
?>
<section class="shell vl-photo-nav" aria-label="<?php esc_attr_e( 'Explore travel sections', 'voyagelight' ); ?>">
	<?php foreach ( vl_photo_nav_items( $nav_cards ) as $item ) : ?>
		<a class="vl-photo-tile" href="<?php echo esc_url( $item[3] ); ?>">
			<img src="<?php echo esc_url( $item[0] ); ?>" alt="" loading="lazy" decoding="async">
			<span class="vl-photo-tile-content"><strong><?php echo esc_html( $item[1] ); ?></strong><small><?php echo esc_html( $item[2] ); ?></small></span>
		</a>
	<?php endforeach; ?>
</section>
