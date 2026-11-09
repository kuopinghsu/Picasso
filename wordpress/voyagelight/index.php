<?php
get_header();
$title = is_home() && get_option( 'page_for_posts' ) ? get_the_title( get_option( 'page_for_posts' ) ) : __( 'All stories', 'voyagelight' );
?>
<main id="main" class="shell section<?php echo 'none' === vl_sidebar_position() ? '' : ' content-layout'; ?>">
	<div class="content-main">
		<div class="section-heading"><div><span class="eyebrow"><?php esc_html_e( 'Journal', 'voyagelight' ); ?></span><h1><?php echo esc_html( $title ); ?></h1></div></div>
		<div class="listing-grid">
			<?php
			if ( have_posts() ) {
				while ( have_posts() ) {
					the_post();
					vl_card( get_the_ID() );
				}
			} else {
				echo '<p>' . esc_html__( 'No stories published yet.', 'voyagelight' ) . '</p>';
			}
			?>
		</div>
		<div class="pagination"><?php the_posts_pagination(); ?></div>
	</div>
	<?php
	if ( 'none' !== vl_sidebar_position() ) {
		get_sidebar();
	}
	?>
</main>
<?php
get_footer();
