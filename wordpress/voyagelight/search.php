<?php
get_header();
?>
<main id="main" class="shell section">
	<?php /* translators: %s: the words searched for. */ ?>
	<div class="section-heading"><h1><?php echo esc_html( sprintf( __( 'Search results for: %s', 'voyagelight' ), get_search_query() ) ); ?></h1></div>
	<div class="listing-grid">
		<?php
		if ( have_posts() ) {
			while ( have_posts() ) {
				the_post();
				vl_card( get_the_ID() );
			}
		} else {
			echo '<p>' . esc_html__( 'No results found.', 'voyagelight' ) . '</p>';
			get_search_form();
		}
		?>
	</div>
	<?php the_posts_pagination(); ?>
</main>
<?php
get_footer();
