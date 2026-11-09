<?php
get_header();
?>
<main id="main" class="shell section notfound">
	<span class="eyebrow"><?php esc_html_e( '404 · off the map', 'voyagelight' ); ?></span>
	<h1><?php esc_html_e( 'This trail ends here.', 'voyagelight' ); ?></h1>
	<p><?php esc_html_e( 'The page you’re looking for may have moved.', 'voyagelight' ); ?></p>
	<a class="button primary" href="<?php echo esc_url( home_url( '/' ) ); ?>"><?php esc_html_e( 'Return to homepage', 'voyagelight' ); ?></a>
	<?php get_search_form(); ?>
</main>
<?php
get_footer();
