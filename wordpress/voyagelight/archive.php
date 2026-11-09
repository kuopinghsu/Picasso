<?php
get_header();
?>
<main id="main" class="shell section<?php echo 'none' === vl_sidebar_position() ? '' : ' content-layout'; ?>">
	<div class="content-main">
		<div class="section-heading"><div><?php echo vl_breadcrumbs(); // phpcs:ignore WordPress.Security.EscapeOutput.OutputNotEscaped -- escaped in vl_breadcrumbs(). ?><span class="eyebrow"><?php esc_html_e( 'Explore the archive', 'voyagelight' ); ?></span><h1><?php the_archive_title(); ?></h1><?php the_archive_description( '<p>', '</p>' ); ?></div></div>
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
	<?php
	if ( 'none' !== vl_sidebar_position() ) {
		get_sidebar();
	}
	?>
</main>
<?php
get_footer();
