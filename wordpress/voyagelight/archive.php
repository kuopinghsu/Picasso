<?php
get_header();
?>
<main id="main" class="shell section content-layout">
	<div class="content-main">
		<div class="section-heading"><div><span class="eyebrow"><?php esc_html_e( 'EXPLORE THE ARCHIVE', 'voyagelight' ); ?></span><h1><?php the_archive_title(); ?></h1><?php the_archive_description( '<p>', '</p>' ); ?></div></div>
		<div class="listing-grid">
			<?php
			if ( have_posts() ) {
				while ( have_posts() ) {
					the_post();
					vl_card( get_the_ID() );
				}
			} else {
				echo '<p>' . esc_html__( 'No stories in this collection yet.', 'voyagelight' ) . '</p>';
			}
			?>
		</div>
		<?php the_posts_pagination(); ?>
	</div>
	<?php get_sidebar(); ?>
</main>
<?php
get_footer();
