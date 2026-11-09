<?php
get_header();
?>
<main id="main" class="shell section">
	<?php /* translators: %s: the words searched for. */ ?>
	<div class="section-heading"><h1><?php echo esc_html( sprintf( __( 'Search results for: %s', 'voyagelight' ), get_search_query() ) ); ?></h1></div>
	<?php if ( have_posts() ) : ?>
		<div class="listing-grid">
			<?php
			while ( have_posts() ) {
				the_post();
				vl_card( get_the_ID() );
			}
			?>
		</div>
		<?php the_posts_pagination(); ?>
	<?php else : ?>
		<div class="search-empty">
			<p><?php esc_html_e( 'No results found.', 'voyagelight' ); ?></p>
			<p><?php esc_html_e( 'Try other words, or browse the travel categories.', 'voyagelight' ); ?></p>
			<?php
			get_search_form();
			// The top travel categories with stories, so the reader has somewhere to go next.
			$vl_terms = get_terms(
				array(
					'taxonomy'   => 'category',
					'parent'     => 0,
					'hide_empty' => true,
					'exclude'    => array( (int) get_option( 'default_category' ) ),
					'orderby'    => 'count',
					'order'      => 'DESC',
					'number'     => 12,
				)
			);
			if ( $vl_terms && ! is_wp_error( $vl_terms ) ) {
				echo '<nav aria-labelledby="vl-search-browse"><h2 id="vl-search-browse">' . esc_html__( 'Travel categories', 'voyagelight' ) . '</h2><ul class="search-chips">';
				foreach ( $vl_terms as $vl_term ) {
					echo '<li><a href="' . esc_url( get_term_link( $vl_term ) ) . '">' . esc_html( $vl_term->name ) . '</a></li>';
				}
				echo '</ul></nav>';
			}
			?>
		</div>
	<?php endif; ?>
</main>
<?php
get_footer();
