<footer class="footer">
	<div class="shell footer-inner">
		<div><b><?php bloginfo( 'name' ); ?></b><p><?php esc_html_e( 'Travel stories, places, and the art of seeing.', 'voyagelight' ); ?></p></div>
		<nav aria-label="<?php esc_attr_e( 'Footer navigation', 'voyagelight' ); ?>">
			<?php
			if ( has_nav_menu( 'footer' ) ) {
				wp_nav_menu( array( 'theme_location' => 'footer', 'container' => false, 'menu_class' => 'nav-list' ) );
			}
			?>
		</nav>
		<small>© <?php echo esc_html( wp_date( 'Y' ) ); ?> <?php bloginfo( 'name' ); ?> · <?php esc_html_e( 'Powered by WordPress', 'voyagelight' ); ?></small>
	</div>
</footer>
<?php wp_footer(); ?>
</body>
</html>
