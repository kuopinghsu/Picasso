<?php
/**
 * The start page's collections section; front-page.php draws the sections in the order set in the Customizer.
 *
 * @package VoyageLight
 */

$to_section = $args['to_section'];
$stories_url = $args['stories_url'];
?>
<section class="shell section editorial-grid">
	<div class="editorial-panel">
		<span class="eyebrow"><?php esc_html_e( 'Collections', 'voyagelight' ); ?></span>
		<h2><?php esc_html_e( 'Every place has a story.', 'voyagelight' ); ?></h2>
		<p><?php esc_html_e( 'Follow the route, the local culture, and the little details along the way. Build a travel journal with your own voice.', 'voyagelight' ); ?></p>
		<a href="<?php echo esc_url( $to_section( 'stories', 'stories', $stories_url ) ); ?>" class="text-link"><?php esc_html_e( 'Explore the journal ↗', 'voyagelight' ); ?></a>
	</div>
	<?php $editorial_photo = get_theme_mod( 'vl_editorial_image', '' ); ?>
	<div class="editorial-photo">
		<?php if ( $editorial_photo ) : ?>
			<img src="<?php echo esc_url( $editorial_photo ); ?>" alt="" loading="lazy">
		<?php else : ?>
			<img src="<?php echo esc_url( vl_image( 'japan' ) ); ?>" alt="" loading="lazy">
		<?php endif; ?>
	</div>
</section>
