<footer class="footer">
	<div class="shell footer-inner">
		<div class="footer-brand"><b><?php bloginfo( 'name' ); ?></b><p><?php esc_html_e( 'Travel stories, places, and the art of seeing.', 'voyagelight' ); ?></p></div>
		<?php
		// The widgets from Appearance > Widgets > Footer as the footer's columns, or else
		// the top-level categories with stories and the latest Journeys.
		if ( is_active_sidebar( 'footer-1' ) ) :
			dynamic_sidebar( 'footer-1' );
		else :
		$footer_children = vl_category_children();
		$footer_skip     = (int) get_option( 'default_category' );
		$footer_links    = '';
		foreach ( array_slice( isset( $footer_children[0] ) ? $footer_children[0] : array(), 0, 8 ) as $footer_term ) {
			$footer_url = get_term_link( $footer_term );
			if ( $footer_term->term_id !== $footer_skip && ! is_wp_error( $footer_url ) ) {
				$footer_links .= '<li><a href="' . esc_url( $footer_url ) . '">' . esc_html( $footer_term->name ) . '</a></li>';
			}
		}
		if ( '' !== $footer_links ) {
			echo '<nav class="footer-col" aria-label="' . esc_attr__( 'Travel categories', 'voyagelight' ) . '"><h2>' . esc_html__( 'Travel categories', 'voyagelight' ) . '</h2><ul>' . $footer_links . '</ul></nav>'; // phpcs:ignore WordPress.Security.EscapeOutput.OutputNotEscaped -- escaped above.
		}
		$footer_trips = get_posts(
			array(
				'post_type'      => 'vl_trip',
				'post_status'    => 'publish',
				'posts_per_page' => vl_home_count( 'footer', 5 ),
				'no_found_rows'  => true,
				'orderby'        => array(
					'date' => 'DESC',
					'ID'   => 'DESC',
				),
			)
		);
		if ( $footer_trips ) {
			echo '<nav class="footer-col" aria-label="' . esc_attr__( 'Journeys', 'voyagelight' ) . '"><h2>' . esc_html__( 'Journeys', 'voyagelight' ) . '</h2><ul>';
			foreach ( $footer_trips as $footer_trip ) {
				echo '<li><a href="' . esc_url( get_permalink( $footer_trip ) ) . '">' . esc_html( get_the_title( $footer_trip ) ) . '</a></li>';
			}
			echo '</ul></nav>';
		}
		endif;
		?>
	</div>
	<div class="shell footer-bottom">
		<?php
		// A text emptied in Appearance > Customize > VoyageLight text takes its separator with it.
		$footer_powered = __( 'Powered by WordPress', 'voyagelight' );
		?>
		<small>© <?php echo esc_html( wp_date( 'Y' ) ); ?> <?php bloginfo( 'name' ); ?><?php echo '' !== $footer_powered ? ' · ' . esc_html( $footer_powered ) : ''; ?></small>
		<?php if ( has_nav_menu( 'footer' ) ) : ?>
			<nav aria-label="<?php esc_attr_e( 'Footer navigation', 'voyagelight' ); ?>">
				<?php wp_nav_menu( array( 'theme_location' => 'footer', 'container' => false, 'menu_class' => 'nav-list', 'depth' => 1 ) ); ?>
			</nav>
		<?php endif; ?>
	</div>
</footer>
<?php wp_footer(); ?>
</body>
</html>
